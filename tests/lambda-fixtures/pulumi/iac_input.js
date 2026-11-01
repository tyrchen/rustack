"use strict";
const support = require("./program_support");
const { check } = require("./iac_runtime");

function input(event, requestId) {
    check(event && typeof event === "object" && !Array.isArray(event) &&
        Buffer.byteLength(JSON.stringify(event)) <= 8192 && Object.keys(event).length === 5 &&
        Object.keys(event).every(key => ["mode", "nonce", "table", "businessArn", "target"].includes(key)));
    check(["lifecycle", "stop"].includes(event.mode) && typeof event.table === "string" &&
        /^[A-Za-z0-9_.-]{3,128}$/.test(event.table));
    check(typeof requestId === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(requestId));
    const workspace = "/tmp/iac-" + requestId;
    const config = support.validate({ nonce: event.nonce, endpoint: process.env.AWS_ENDPOINT_URL,
        businessArn: event.businessArn, target: event.target, role: "arn:aws:iam::000000000000:role/ImageIac",
        kmsKey: "arn:aws:kms:us-east-1:000000000000:key/00000000-0000-4000-8000-000000000000",
        emptyConfig: workspace + "/empty-config" });
    return { event, config, workspace, requestId };
}

module.exports = { input };
