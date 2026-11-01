"use strict";

const fs = require("node:fs/promises");
const { randomBytes, createHash } = require("node:crypto");
const path = require("node:path");
const { performance } = require("node:perf_hooks");
const { setTimeout: delay } = require("node:timers/promises");
const { DynamoDBClient, PutItemCommand, GetItemCommand } = require("@aws-sdk/client-dynamodb");
const runtime = require("./iac_runtime");
const support = require("./program_support");
const { check, fresh, bytes } = runtime;
const { CommandFailure } = require("./child_owner");
const PROJECT = "rustack-iac";
const STACK = "fixture";

const { input } = require("./iac_input");

class Session {
    constructor(data, deadline, versions) {
        Object.assign(this, data);
        this.deadline = deadline;
        this.versions = versions;
        this.ordinal = 0;
        this.outputBytes = 0;
        this.records = [];
        this.client = new DynamoDBClient({ maxAttempts: 1 });
    }
    async send(command) {
        fresh(this.deadline);
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), Math.min(4000, this.deadline - performance.now()));
        try {
            const value = await this.client.send(command, { abortSignal: controller.signal });
            fresh(this.deadline);
            return value;
        } finally { clearTimeout(timer); controller.abort(); }
    }
    async setup() {
        await fs.mkdir(this.workspace, { mode: 0o700 });
        for (const name of ["project", "home", "backend", "logs"]) await fs.mkdir(path.join(this.workspace, name), { mode: 0o700 });
        await fs.symlink(runtime.TOOLS + "/plugins", this.workspace + "/home/plugins");
        await fs.symlink(__dirname + "/node_modules", this.workspace + "/project/node_modules");
        for (const name of ["program.js", "program_support.js"]) await fs.copyFile(path.join(__dirname, name), this.workspace + "/project/" + name);
        await fs.writeFile(this.workspace + "/project/Pulumi.yaml", "name: " + PROJECT + "\nruntime:\n  name: nodejs\n  options:\n    typescript: false\nmain: program.js\n", { flag: "wx", mode: 0o600 });
        await fs.writeFile(this.workspace + "/project/package.json", '{"private":true,"main":"program.js"}', { flag: "wx", mode: 0o600 });
        await fs.writeFile(this.workspace + "/empty-config", "", { flag: "wx", mode: 0o600 });
        await fs.writeFile(this.workspace + "/config.json", JSON.stringify(this.config), { flag: "wx", mode: 0o600 });
        const key = process.env.AWS_ACCESS_KEY_ID, secret = process.env.AWS_SECRET_ACCESS_KEY;
        check(typeof key === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(key) &&
            typeof secret === "string" && /^[\x21-\x7e]{1,256}$/.test(secret));
        this.environment = {
            PATH: "/opt/pulumi/bin:/var/lang/bin:/usr/bin:/bin", LANG: "C.UTF-8",
            USER: "rustack-iac", LOGNAME: "rustack-iac",
            NODE_PATH: __dirname + "/node_modules:/var/runtime/node_modules:/var/lang/lib/node_modules",
            GOMAXPROCS: "1", UV_THREADPOOL_SIZE: "2",
            AWS_ACCESS_KEY_ID: key, AWS_SECRET_ACCESS_KEY: secret, AWS_REGION: "us-east-1",
            AWS_DEFAULT_REGION: "us-east-1", AWS_EC2_METADATA_DISABLED: "true",
            AWS_CONFIG_FILE: this.config.emptyConfig, AWS_SHARED_CREDENTIALS_FILE: this.config.emptyConfig,
            AWS_ENDPOINT_URL: this.config.endpoint, AWS_IGNORE_CONFIGURED_ENDPOINT_URLS: "false",
            PULUMI_HOME: this.workspace + "/home", PULUMI_BACKEND_URL: "file://" + this.workspace + "/backend",
            PULUMI_CONFIG_PASSPHRASE: randomBytes(32).toString("hex"), PULUMI_SKIP_UPDATE_CHECK: "true",
            PULUMI_DISABLE_AUTOMATIC_PLUGIN_ACQUISITION: "true", PULUMI_IGNORE_AMBIENT_PLUGINS: "true",
            RUSTACK_IAC_CONFIG: this.workspace + "/config.json",
        };
    }
    gateId(ordinal, stage) { return "gate:" + this.event.nonce + ":" + this.event.mode + ":" + ordinal + ":" + stage; }
    async record(stage, details = {}) {
        const row = { nonce: this.event.nonce, mode: this.event.mode, stage, ordinal: this.ordinal,
            requestId: this.requestId, targetArn: "arn:aws:states:us-east-1:000000000000:stateMachine:" + this.config.target,
            ...details };
        const payload = JSON.stringify(row);
        const item = { id: { S: this.event.nonce + ":" + this.event.mode + ":" + stage }, payload: { S: payload } };
        check(Buffer.byteLength(JSON.stringify(item)) <= 2048);
        await this.send(new PutItemCommand({ TableName: this.event.table, Item: item }));
    }
    async grant(stage, deadline, alive = () => true) {
        deadline = Math.min(deadline, this.deadline);
        while (performance.now() < deadline && alive()) {
            const result = await this.send(new GetItemCommand({ TableName: this.event.table,
                Key: { id: { S: this.gateId(this.ordinal, stage) } }, ConsistentRead: true }));
            fresh(deadline);
            if (result.Item) {
                check(Object.keys(result.Item).length === 2 && result.Item.id?.S === this.gateId(this.ordinal, stage) &&
                    result.Item.open?.BOOL === true);
                return;
            }
            await delay(Math.min(100, Math.max(1, deadline - performance.now())));
        }
        throw new Error("IaC host grant expired");
    }
    async held(stage, owner) {
        const file = support.stagePath(this.workspace, owner.ordinal, stage, false);
        const phase = await runtime.waitFile(file, owner.deadline, owner.alive);
        check(Object.keys(phase).length === 4 && phase.nonce === this.config.nonce && phase.stage === stage &&
            phase.ordinal === owner.ordinal && phase.arn === "arn:aws:states:us-east-1:000000000000:stateMachine:" + this.config.target);
        const processes = await runtime.processes(this.workspace);
        check(owner.alive() && processes.some(value => value.cli && value.pid === owner.pid) &&
            processes.some(value => value.language) && processes.some(value => value.provider));
        await this.record(stage, { processes, versions: this.versions });
        await this.grant(stage, owner.deadline, owner.alive);
        check(owner.alive());
        await support.publishJSON(support.stagePath(this.workspace, owner.ordinal, stage, true), {
            nonce: this.config.nonce, ordinal: owner.ordinal, stage,
        }, owner.deadline);
    }
    async commandFailure(label, code, stdout, stderr, cause) {
        const text = Buffer.concat([stdout, stderr]).toString("utf8");
        const patterns = [
            ["module-missing", /Cannot find module|MODULE_NOT_FOUND/],
            ["plugin-missing", /plugin.*not found|could not find.*plugin|Missing.*plugin/i],
            ["home-missing", /HOME|home directory/], ["user-missing", /unknown user|userid|user: Current/],
            ["permission", /permission denied|EACCES/], ["readonly", /read-only|EROFS/],
            ["syntax", /SyntaxError|Unexpected token/], ["invalid-flag", /unknown flag/],
            ["provider-validation", /ValidationException/], ["connection", /connection refused|ENOTFOUND|network is unreachable/i],
        ];
        const category = patterns.find(([, pattern]) => pattern.test(text))?.[0] ??
            (cause?.message === "IaC command did not reach its live gate" ? "gate-not-reached" :
                cause?.message === "bounded IaC runtime validation failed" ? "gate-validation" : "other");
        await this.record("command-failed", { label, code: Number.isInteger(code) && code >= 0 && code <= 255 ? code : null, category });
        await this.grant("failure-audit", Math.min(this.deadline, performance.now() + 20000));
    }
    async command(label, args, value = 23, unsupported = "none", gate = "none") {
        await this.record("command-start", { label });
        let result;
        try {
            result = await runtime.run(this, label, args, { ...this.environment,
            RUSTACK_IAC_VALUE: String(value), RUSTACK_IAC_UNSUPPORTED: unsupported, RUSTACK_IAC_GATE: gate },
            gate === "none" ? undefined : owner => this.held(gate, owner));
        } catch (error) {
            if (error instanceof CommandFailure) {
                await this.commandFailure(label, error.code, error.stdout, error.stderr, error.cause);
            }
            throw error;
        }
        const diagnostic = new TextDecoder("utf-8", { fatal: true }).decode(Buffer.concat([result.stdout, result.stderr]));
        await this.record("command-end", { label, code: result.code, validationException: diagnostic.includes("ValidationException") });
        const accepted = unsupported === "none" ? result.code === 0 : result.code !== 0 && diagnostic.includes("ValidationException");
        if (!accepted) await this.commandFailure(label, result.code, result.stdout, result.stderr);
        check(accepted);
        const hash = data => createHash("sha256").update(data).digest("hex");
        this.records.push({ ordinal: result.ordinal, label, code: result.code,
            stdoutSha256: hash(result.stdout), stderrSha256: hash(result.stderr) });
        check(this.records.length <= 20);
        return diagnostic;
    }
    async checkpoint(stage, present) {
        const relative = "backend/.pulumi/stacks/" + PROJECT + "/" + STACK + ".json";
        let actual;
        try { actual = await bytes(this.workspace + "/" + relative, 2 * 1024 * 1024); }
        catch (error) { check(!present && error.code === "ENOENT"); }
        check(present === (actual !== undefined));
        const checkpoint = { path: relative, present, bytes: actual?.length ?? 0,
            sha256: actual ? createHash("sha256").update(actual).digest("hex") : "" };
        await this.record(stage, { checkpoint, commands: this.records.map(({ ordinal, label, code }) => ({ ordinal, label, code })) });
        await this.grant(stage, Math.min(this.deadline, performance.now() + 60000));
    }
    async lifecycle() {
        const shared = ["--stack", STACK, "--non-interactive", "--color", "never"];
        const up = ["up", "--yes", "--skip-preview", ...shared];
        const refresh = ["refresh", "--yes", "--expect-no-changes", ...shared];
        const version = await this.command("version", ["version"]);
        check(version.trim() === "v3.273.0");
        await this.command("login", ["login", this.environment.PULUMI_BACKEND_URL, "--non-interactive"]);
        await this.command("init", ["stack", "init", STACK, "--secrets-provider", "passphrase", "--non-interactive"]);
        await this.command("create", up, 21, "none", "created");
        // The Stop resource never grants its initial up; this path is positive only.
        check(this.event.mode === "lifecycle");
        await this.command("refresh-created", refresh, 21);
        await this.record("refreshed-created");
        await this.grant("refreshed-created", Math.min(this.deadline, performance.now() + 60000));
        await this.command("update", up, 23, "none", "updated");
        await this.command("refresh-updated", refresh);
        await this.record("refreshed-updated");
        await this.grant("refreshed-updated", Math.min(this.deadline, performance.now() + 60000));
        for (const unsupported of ["publish", "logging", "encryption"]) {
            await this.command("reject-" + unsupported, up, 23, unsupported);
            await this.record("rejected-" + unsupported);
            await this.grant("rejected-" + unsupported, Math.min(this.deadline, performance.now() + 60000));
            await this.command("recover-" + unsupported, up);
            const diagnostic = await this.command("settle-" + unsupported, [...up, "--expect-no-changes"]);
            check(diagnostic.includes("unchanged"));
            await this.record("recovered-" + unsupported);
            await this.grant("recovered-" + unsupported, Math.min(this.deadline, performance.now() + 60000));
        }
        await this.checkpoint("pre-destroy", true);
        await this.command("destroy", ["destroy", "--yes", ...shared]);
        await this.checkpoint("post-destroy", true);
        await this.command("remove", ["stack", "rm", STACK, "--yes", "--non-interactive"]);
        await this.checkpoint("post-removal", false);
        check(this.ordinal === 18);
        await this.record("finished", { versions: this.versions, commandCount: this.ordinal });
        return { requestId: this.requestId, versions: this.versions, commandCount: this.ordinal };
    }
}

module.exports = { input, Session };
