"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { once } = require("node:events");
const { performance } = require("node:perf_hooks");
const CAP = 16 * 1024 * 1024;
function check(value) { if (!value) throw new Error("bounded evidence reader failed"); }
function octal(header, start, length, value) {
    const text = value.toString(8).padStart(length - 1, "0") + "\0";
    check(text.length === length);
    header.write(text, start, length, "ascii");
}
function header(name, size, directory) {
    const output = Buffer.alloc(512);
    check(Buffer.byteLength(name) <= 256);
    const split = Buffer.byteLength(name) <= 100 ? -1 : name.lastIndexOf("/");
    const prefix = split < 0 ? "" : name.slice(0, split);
    const suffix = split < 0 ? name : name.slice(split + 1);
    check(Buffer.byteLength(prefix) <= 155 && Buffer.byteLength(suffix) <= 100);
    output.write(suffix, 0, 100, "utf8");
    output.write(prefix, 345, 155, "utf8");
    octal(output, 100, 8, directory ? 0o700 : 0o600);
    octal(output, 108, 8, 1000); octal(output, 116, 8, 1000);
    octal(output, 124, 12, size); octal(output, 136, 12, 0);
    output.fill(32, 148, 156); output[156] = directory ? 53 : 48;
    output.write("ustar\0", 257, 6, "ascii"); output.write("00", 263, 2, "ascii");
    const checksum = output.reduce((total, value) => total + value, 0);
    output.write(checksum.toString(8).padStart(6, "0") + "\0 ", 148, 8, "ascii");
    return output;
}
async function read(workspace, root, write, deadline) {
    check(/^\/tmp\/iac-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(workspace));
    check(["backend", "logs"].includes(root));
    const base = fs.lstatSync(workspace);
    check(base.isDirectory() && !base.isSymbolicLink() && base.uid === process.getuid());
    let total = 0, entries = 0;
    async function emit(value) {
        check(performance.now() < deadline);
        total += value.length;
        check(total <= CAP);
        await write(value);
        check(performance.now() < deadline);
    }
    async function visit(relative, depth) {
        check(++entries <= 128 && depth <= 8 && Buffer.byteLength(relative) <= 256 &&
            /^[A-Za-z0-9_.\/-]+$/.test(relative) && relative.split("/").every(value => !["", ".", ".."].includes(value)));
        const file = path.join(workspace, relative), stat = fs.lstatSync(file);
        check(!stat.isSymbolicLink() && stat.uid === process.getuid());
        if (stat.isDirectory()) {
            await emit(header(relative, 0, true));
            const directory = fs.opendirSync(file), names = [];
            try {
                let entry;
                while ((entry = directory.readSync()) !== null) {
                    check(entries + names.length < 128);
                    names.push(entry.name);
                }
            } finally { directory.closeSync(); }
            for (const name of names.sort()) await visit(relative + "/" + name, depth + 1);
        } else {
            check(stat.isFile() && stat.nlink === 1 && stat.size <= 2 * 1024 * 1024);
            const descriptor = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
            let content;
            try {
                const opened = fs.fstatSync(descriptor);
                check(opened.isFile() && opened.ino === stat.ino && opened.dev === stat.dev && opened.size === stat.size);
                content = Buffer.alloc(opened.size + 1);
                let offset = 0, received;
                while ((received = fs.readSync(descriptor, content, offset, content.length - offset, null)) !== 0) {
                    offset += received;
                    check(offset <= opened.size);
                }
                const after = fs.fstatSync(descriptor);
                check(offset === opened.size && after.size === opened.size && after.mtimeMs === opened.mtimeMs);
                content = content.subarray(0, offset);
            } finally { fs.closeSync(descriptor); }
            await emit(header(relative, content.length, false));
            await emit(content);
            if (content.length % 512) await emit(Buffer.alloc(512 - content.length % 512));
        }
    }
    await visit(root, 1);
    await emit(Buffer.alloc(1024));
}
async function main() {
    check(process.getuid() === 1000 && process.getgid() === 1000 && process.argv.length === 4);
    const requestId = process.argv[2], root = process.argv[3];
    await read("/tmp/iac-" + requestId, root, async value => {
        if (!process.stdout.write(value)) await once(process.stdout, "drain");
    }, performance.now() + 10000);
}
if (require.main === module) main().catch(() => { process.stderr.write("bounded evidence reader failed\n"); process.exitCode = 1; });
module.exports = { read };
