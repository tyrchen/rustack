"use strict";

const fs = require("node:fs");
const { randomUUID } = require("node:crypto");

let counter = 0;

class NodeFixtureError extends Error {
    constructor() {
        super("intentional Node fixture failure");
        this.name = "NodeFixtureError";
    }
}

function filesystem() {
    const target = "/var/task/node-write-probe";
    let rootReadOnly = false;
    try {
        const fd = fs.openSync(target, "wx", 0o600);
        fs.closeSync(fd);
        fs.unlinkSync(target);
    } catch (error) {
        if (error.code !== "EROFS") throw error;
        rootReadOnly = true;
    }
    if (!rootReadOnly) throw new Error("Node fixture root is writable");
    const temporary = "/tmp/node-" + randomUUID();
    try {
        fs.writeFileSync(temporary, "node-private-roundtrip", { mode: 0o600, flag: "wx" });
        return {
            rootReadOnly,
            temporaryRoundtrip: fs.readFileSync(temporary, "utf8") === "node-private-roundtrip",
            temporaryMode: fs.statSync(temporary).mode & 0o777,
        };
    } finally {
        fs.unlinkSync(temporary);
    }
}

async function compute(event, context, multiplier, handler) {
    if (!event || !Number.isSafeInteger(event.value) || Math.abs(event.value) > 1000000) {
        throw new Error("bounded integer fixture value required");
    }
    counter += 1;
    if (event.fail) throw new NodeFixtureError();
    if (event.logs) {
        console.log("NODE_STDOUT_CAPTURE");
        console.error("NODE_STDERR_CAPTURE");
    }
    return {
        computed: event.value * multiplier,
        counter,
        runtime: "node24",
        nodeVersion: process.versions.node,
        architecture: process.arch,
        uid: process.getuid(),
        requestId: context.awsRequestId,
        functionName: context.functionName,
        functionVersion: context.functionVersion,
        invokedArn: context.invokedFunctionArn,
        memoryMb: context.memoryLimitInMB,
        configuredValue: process.env.TEST_VALUE,
        handler,
        ...filesystem(),
    };
}

exports.handler = async (event, context) => compute(event, context, 2, "handler");
exports.alternate = async (event, context) => compute(event, context, 3, "alternate");
