"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const runtime = require("./iac_runtime");
const actualInput = require("./iac_input");

test("test_should_reject_malformed_payload_before_any_provenance_io", async () => {
    let reads = 0;
    const module = { exports: {} };
    vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../iac.js"), "utf8"), {
        exports: module.exports,
        require(name) {
            if (name === "node:perf_hooks") return require(name);
            if (name === "./pulumi/iac_runtime") return { ...runtime, source: async () => { reads += 1; throw new Error("unexpected IO"); } };
            if (name === "./pulumi/iac_input") return actualInput;
            throw new Error("unexpected prevalidation dependency");
        },
    });
    const context = { getRemainingTimeInMillis: () => 720000, awsRequestId: "00000000-0000-4000-8000-000000000000" };
    for (const event of [null, [], {}, { mode: "probe", extra: true }, { mode: "lifecycle" },
        { mode: "lifecycle", nonce: "x".repeat(8192), table: "abc", businessArn: "bad", target: "abc" }]) {
        await assert.rejects(module.exports.handler(event, context), { name: "IacFixtureFailure" });
    }
    await assert.rejects(module.exports.handler({ mode: "probe" }, { ...context, awsRequestId: "bad" }));
    assert.equal(reads, 0);
});
