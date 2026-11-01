"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const { mkdtemp, mkdir, writeFile, rm } = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const { setImmediate: immediate } = require("node:timers/promises");
const { Worker } = require("node:worker_threads");
const test = require("node:test");
const Arborist = require("@npmcli/arborist");
const mapWorkspaces = require("@npmcli/map-workspaces");
const packlist = require("npm-packlist");
const glob = require("glob");
const minimatch = require("minimatch");

async function workspace(run) {
    const root = await mkdtemp(path.join(os.tmpdir(), "rustack-iac-dependencies-"));
    try {
        const pkg = { name: "iac-workspace", version: "1.0.0", private: true, workspaces: ["packages/*"] };
        await writeFile(path.join(root, "package.json"), JSON.stringify(pkg));
        for (const name of ["a", "b"]) {
            const directory = path.join(root, "packages", name);
            await mkdir(directory, { recursive: true });
            await writeFile(path.join(directory, "package.json"), JSON.stringify({ name: "iac-" + name, version: "1.0.0" }));
            await writeFile(path.join(directory, "index.js"), '"use strict";\n');
        }
        return await run(root, pkg);
    } finally {
        await rm(root, { recursive: true });
    }
}

test("test_should_load_the_exact_patched_sdk_commonjs_exports", () => {
    for (const [name, expected] of Object.entries({
        "@pulumi/pulumi": "3.273.0", "@pulumi/aws": "7.52.1", glob: "14.0.3", minimatch: "11.0.3",
    })) assert.equal(require(name + "/package.json").version, expected);
    assert.equal(typeof require("@pulumi/pulumi").secret, "function");
    assert.equal(typeof require("@pulumi/aws").sfn.StateMachine, "function");
    assert.equal(typeof glob.Glob, "function");
    assert.equal(typeof minimatch.Minimatch, "function");
});

test("test_should_map_actual_workspaces_with_patched_glob_and_minimatch", async () => workspace(async (root, pkg) => {
    const expected = ["packages/a/package.json", "packages/b/package.json"];
    assert.deepEqual(glob.globSync("packages/*/package.json", { cwd: root }).sort(), expected);
    assert.deepEqual((await glob.glob("packages/*/package.json", { cwd: root })).sort(), expected);
    const mapped = await mapWorkspaces({ cwd: root, pkg });
    assert.deepEqual([...mapped.keys()].sort(), ["iac-a", "iac-b"]);
    const matcher = new minimatch.Minimatch("packages/*/package.json");
    assert(expected.every(value => matcher.match(value)));
    assert.equal(matcher.match("packages/a/index.js"), false);
}));

test("test_should_load_the_real_dependency_graph_and_pack_workspace_files", async () => {
    const tree = await new Arborist({ path: __dirname }).loadActual();
    assert.equal(tree.edgesOut.get("@pulumi/aws").to.package.version, "7.52.1");
    assert.equal(tree.edgesOut.get("@pulumi/pulumi").to.package.version, "3.273.0");
    await workspace(async root => {
        const local = await new Arborist({ path: root }).loadActual();
        const files = await packlist(local);
        assert(files.includes("package.json"));
        assert(files.includes("packages/a/index.js"));
        assert(files.includes("packages/b/index.js"));
        assert(files.every(value => !value.split(path.sep).includes("node_modules")));
    });
});

test("test_should_abort_glob_before_directory_traversal_and_remain_stopped", async () => workspace(async root => {
    let reads = 0;
    const observed = { ...fs, readdir(...args) { reads += 1; return fs.readdir(...args); } };
    const controller = new AbortController();
    controller.abort(new Error("fixture walk aborted"));
    await assert.rejects(glob.glob("**/*", { cwd: root, fs: observed, signal: controller.signal }), /fixture walk aborted/);
    await immediate();
    await immediate();
    assert.equal(reads, 0);
}));

test("test_should_bound_amplifying_matcher_work_in_an_owned_worker", async () => {
    const source = `
        const { parentPort, workerData } = require("node:worker_threads");
        const { Minimatch } = require(workerData.module);
        const matcher = new Minimatch("**/../".repeat(64) + "leaf", { optimizationLevel: 2 });
        parentPort.postMessage({ matched: matcher.match("leaf"), patterns: matcher.set.length });
    `;
    const worker = new Worker(source, { eval: true, workerData: { module: require.resolve("minimatch") } });
    let timer;
    try {
        const result = await new Promise((resolve, reject) => {
            timer = setTimeout(() => reject(new Error("bounded matcher observation expired")), 3000);
            worker.once("message", resolve);
            worker.once("error", reject);
            worker.once("exit", code => { if (code !== 0) reject(new Error("matcher worker failed")); });
        });
        assert.equal(typeof result.matched, "boolean");
        assert(Number.isSafeInteger(result.patterns) && result.patterns <= 256);
    } finally {
        clearTimeout(timer);
        await worker.terminate();
    }
});
