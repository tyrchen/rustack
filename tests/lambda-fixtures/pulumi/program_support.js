"use strict";

const fs = require("node:fs/promises");
const { constants } = require("node:fs");
const path = require("node:path");
const { performance } = require("node:perf_hooks");
const { randomUUID } = require("node:crypto");
const { setTimeout: delay } = require("node:timers/promises");

function check(value) {
    if (!value) throw new Error("invalid bounded IaC program input");
}
function validate(config) {
    check(config && typeof config === "object" && !Array.isArray(config));
    check(Object.keys(config).length === 7 && Object.keys(config).every(key =>
        ["nonce", "endpoint", "businessArn", "target", "role", "kmsKey", "emptyConfig"].includes(key)));
    check(typeof config.nonce === "string" && /^[0-9a-f]{32}$/.test(config.nonce));
    const origin = new URL(config.endpoint);
    check(origin.protocol === "http:" && !origin.username && !origin.password && origin.pathname === "/" &&
        !origin.search && !origin.hash && config.endpoint.length <= 256);
    check(/^arn:aws:lambda:us-east-1:000000000000:function:[A-Za-z0-9_-]{1,64}$/.test(config.businessArn));
    check(typeof config.target === "string" && /^[A-Za-z0-9_-]{1,80}$/.test(config.target));
    check(/^arn:aws:iam::000000000000:role\/[A-Za-z0-9_-]{1,64}$/.test(config.role));
    check(/^arn:aws:kms:us-east-1:000000000000:key\/[0-9a-f-]{36}$/.test(config.kmsKey));
    check(typeof config.emptyConfig === "string" && config.emptyConfig.length <= 512 &&
        /^\/tmp\/iac-[0-9a-f-]{36}\/empty-config$/.test(config.emptyConfig));
    return config;
}
function resourceArgs(config, value, unsupported) {
    validate(config);
    check(value === 21 || value === 23);
    check(["none", "publish", "logging", "encryption"].includes(unsupported));
    const args = {
        name: config.target, roleArn: config.role, type: "STANDARD",
        definition: JSON.stringify({ StartAt: "Call", States: { Call: {
            Type: "Task", Resource: "arn:aws:states:::lambda:invoke",
            Parameters: { FunctionName: config.businessArn, Payload: { value } },
            ResultSelector: { "computed.$": "$.Payload.computed", "requestId.$": "$.Payload.requestId",
                "nodeVersion.$": "$.Payload.nodeVersion", "architecture.$": "$.Payload.architecture",
                "counter.$": "$.Payload.counter" }, End: true,
        } } }),
        tags: { Fixture: config.nonce, Value: String(value) },
    };
    if (unsupported === "publish") args.publish = true;
    if (unsupported === "logging") args.loggingConfiguration = { level: "ALL", includeExecutionData: false,
        logDestination: "arn:aws:logs:us-east-1:000000000000:log-group:" + config.target + ":*" };
    if (unsupported === "encryption") args.encryptionConfiguration = {
        type: "CUSTOMER_MANAGED_KMS_KEY", kmsKeyId: config.kmsKey,
    };
    return args;
}
function stagePath(workspace, ordinal, stage, grant) {
    check(typeof workspace === "string" && /^\/tmp\/iac-[0-9a-f-]{36}$/.test(workspace));
    check(Number.isSafeInteger(ordinal) && ordinal >= 1 && ordinal <= 20);
    check(["created", "updated"].includes(stage) && typeof grant === "boolean");
    return path.join(workspace, (grant ? "grant-" : "phase-") + ordinal + "-" + stage + ".json");
}
async function smallJSON(file, cap) {
    check(Number.isSafeInteger(cap) && cap >= 1 && cap <= 8192);
    const handle = await fs.open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    try {
        const info = await handle.stat();
        check(info.isFile() && info.size <= cap);
        const buffer = Buffer.alloc(cap + 1);
        let offset = 0;
        while (offset < buffer.length) {
            const { bytesRead } = await handle.read(buffer, offset, buffer.length - offset, null);
            if (!bytesRead) break;
            offset += bytesRead;
        }
        check(offset <= cap);
        return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(buffer.subarray(0, offset)));
    } finally {
        await handle.close();
    }
}
async function publishJSON(file, value, deadline, beforePublish = async () => {}) {
    check(typeof file === "string" && /^\/tmp\/iac-[0-9a-f-]{36}\/(phase|grant)-[0-9]{1,2}-(created|updated)\.json$/.test(file));
    const data = JSON.stringify(value);
    check(Buffer.byteLength(data) <= 512 && Number.isFinite(deadline) && performance.now() < deadline);
    const temporary = file + ".pending-" + randomUUID();
    let owned = false;
    try {
        const handle = await fs.open(temporary, "wx", 0o600);
        owned = true;
        try { await handle.writeFile(data); } finally { await handle.close(); }
        await beforePublish(temporary);
        check(performance.now() < deadline);
        // Link publishes only fully written bytes, and never replaces a grant.
        await fs.link(temporary, file);
        check(performance.now() < deadline);
    } finally { if (owned) await fs.unlink(temporary); }
}

async function gateResource(arn, config, options) {
    validate(config);
    check(typeof arn === "string" && arn.length <= 256 && arn ===
        "arn:aws:states:us-east-1:000000000000:stateMachine:" + config.target);
    check(typeof options.dryRun === "boolean" && ["none", "created", "updated"].includes(options.stage));
    if (options.dryRun || options.stage === "none") return arn;
    check(Number.isFinite(options.deadline) && performance.now() < options.deadline);
    const file = stagePath(options.workspace, options.ordinal, options.stage, false);
    const grant = stagePath(options.workspace, options.ordinal, options.stage, true);
    await publishJSON(file, { nonce: config.nonce, stage: options.stage,
        ordinal: options.ordinal, arn }, options.deadline);
    while (performance.now() < options.deadline) {
        try {
            const value = await smallJSON(grant, 512);
            check(Object.keys(value).length === 3 && value.nonce === config.nonce &&
                value.ordinal === options.ordinal && value.stage === options.stage);
            check(performance.now() < options.deadline);
            return arn;
        } catch (error) {
            if (error.code !== "ENOENT") throw error;
        }
        await delay(Math.min(50, options.deadline - performance.now()));
    }
    throw new Error("IaC program gate expired");
}

module.exports = { validate, resourceArgs, stagePath, smallJSON, gateResource, publishJSON };
