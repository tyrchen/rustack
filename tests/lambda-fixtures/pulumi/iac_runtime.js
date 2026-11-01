"use strict";

const fs = require("node:fs/promises");
const { createReadStream, constants } = require("node:fs");
const { createHash } = require("node:crypto");
const path = require("node:path");
const { execute, CommandFailure } = require("./child_owner");
const { performance } = require("node:perf_hooks");
const { setTimeout: delay } = require("node:timers/promises");

const TOOLS = "/opt/pulumi";
const PROVIDER = "plugins/resource-aws-v7.52.1/pulumi-resource-aws";
const FILES = ["bin/pulumi", "bin/pulumi-language-nodejs", PROVIDER];
const ARCHIVES = {
    arm64: ["c2e1b600b37d9d84551259b6f391f5035e336dfe63c49412f22d855a91995ed5", "1976366b71dc08d8827a04fbb8eaf50a0d1ec5757726e593b6d7c27e7636daf8"],
    x64: ["ec08f86671fd58f0a186540e1cee00066393beaf5c6716b0075f690919800432", "2e705824a2103dd6e3dc66744ba2d55cad55d665476b3ffcd8ce024cf4cab081"],
};
function check(value) { if (!value) throw new Error("bounded IaC runtime validation failed"); }
function fresh(deadline) { check(Number.isFinite(deadline) && performance.now() < deadline); }
async function bytes(file, cap) {
    const handle = await fs.open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    try {
        const info = await handle.stat();
        check(info.isFile() && info.size <= cap);
        const value = Buffer.alloc(cap + 1);
        let offset = 0;
        while (offset < value.length) {
            const { bytesRead } = await handle.read(value, offset, value.length - offset, null);
            if (!bytesRead) break;
            offset += bytesRead;
        }
        check(offset <= cap);
        return value.subarray(0, offset);
    } finally { await handle.close(); }
}
async function hash(file, cap, deadline) {
    const state = createHash("sha256");
    let size = 0;
    for await (const chunk of createReadStream(file, { highWaterMark: 65536 })) {
        fresh(deadline);
        size += chunk.length;
        check(size <= cap);
        state.update(chunk);
    }
    fresh(deadline);
    return { sha256: state.digest("hex"), bytes: size };
}
async function source(deadline) {
    fresh(deadline);
    check(process.getuid() === 1000 && ARCHIVES[process.arch] !== undefined);
    const data = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(await bytes(TOOLS + "/manifest.json", 16384)));
    check(data.schemaVersion === 1 && data.cli === "3.273.0" && data.provider === "7.52.1" &&
        data.architecture === (process.arch === "x64" ? "amd64" : "arm64") &&
        JSON.stringify(data.archives) === JSON.stringify(ARCHIVES[process.arch]) &&
        Array.isArray(data.files) && data.files.length === 3 &&
        new Set(data.files.map(value => value.path)).size === 3);
    for (const entry of data.files) {
        check(FILES.includes(entry.path) && /^[0-9a-f]{64}$/.test(entry.sha256) &&
            Number.isSafeInteger(entry.bytes) && entry.bytes >= 64 && entry.bytes <= 2 * 1024 ** 3);
        const file = path.join(TOOLS, entry.path), info = await fs.lstat(file);
        check(info.isFile() && !info.isSymbolicLink() && info.size === entry.bytes && (info.mode & 0o111) === 0o111);
        const actual = await hash(file, 2 * 1024 ** 3, deadline);
        check(actual.bytes === entry.bytes && actual.sha256 === entry.sha256);
    }
    return { cli: "3.273.0", provider: "7.52.1", sdk: require("@pulumi/pulumi/package.json").version,
        aws: require("@pulumi/aws/package.json").version, node: process.versions.node,
        dynamodb: require("@aws-sdk/client-dynamodb/package.json").version };
}
async function processes(workspace) {
    const entries = await fs.readdir("/proc");
    const pids = entries.filter(value => /^[0-9]+$/.test(value));
    check(pids.length <= 256);
    const found = [];
    for (const pid of pids) {
        try {
            const command = (await bytes("/proc/" + pid + "/cmdline", 4096)).toString("utf8");
            if (!command.includes(TOOLS + "/") && !command.includes(workspace + "/project") &&
                !command.startsWith(workspace + "/home/plugins/")) continue;
            const status = (await bytes("/proc/" + pid + "/stat", 4096)).toString("ascii");
            const fields = status.slice(status.lastIndexOf(")") + 2).trim().split(/\s+/);
            const group = Number(fields[2]);
            check(Number.isSafeInteger(group) && group > 0 && group < 2 ** 31 && found.length < 32);
            found.push({ pid: Number(pid), group, provider: command.includes(PROVIDER),
                language: command.includes("/pulumi-language-nodejs"), cli: command.startsWith(TOOLS + "/bin/pulumi\0") });
        } catch (error) { if (error.code !== "ENOENT" && error.code !== "ESRCH") throw error; }
    }
    return found;
}
async function waitFile(file, deadline, alive) {
    while (performance.now() < deadline && alive()) {
        try { return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(await bytes(file, 8192))); }
        catch (error) { if (error.code !== "ENOENT") throw error; }
        await delay(Math.min(50, deadline - performance.now()));
    }
    throw new Error("IaC command did not reach its live gate");
}
async function run(session, label, arguments_, environment, hold) {
    check(/^[a-z0-9-]{1,32}$/.test(label) && session.ordinal < 20);
    fresh(session.deadline);
    const ordinal = ++session.ordinal;
    const deadline = Math.min(session.deadline, performance.now() + 90000);
    let status, failure;
    try {
        status = await execute(TOOLS + "/bin/pulumi", arguments_, {
        cwd: session.workspace + "/project", deadline,
        env: { ...environment, RUSTACK_IAC_COMMAND: String(ordinal) },
        hold: hold ? state => hold({ ...state, ordinal }) : undefined,
        });
    } catch (error) {
        if (!(error instanceof CommandFailure)) throw error;
        status = error;
        failure = error;
    }
    const { stdout, stderr } = status;
    session.outputBytes += stdout.length + stderr.length;
    check(session.outputBytes <= 16 * 1024 * 1024);
    for (const [suffix, value] of [["stdout", stdout], ["stderr", stderr]]) {
        await fs.writeFile(path.join(session.workspace, "logs", String(ordinal) + "-" + label + "." + suffix), value,
            { flag: "wx", mode: 0o600 });
    }
    if (failure) throw failure;
    check((await processes(session.workspace)).length === 0);
    return { code: status.code, stdout, stderr, ordinal };
}

module.exports = { check, fresh, bytes, hash, source, processes, waitFile, run, TOOLS };
