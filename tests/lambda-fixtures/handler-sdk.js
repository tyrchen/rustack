"use strict";

const assert = require("node:assert/strict");
const { performance } = require("node:perf_hooks");
const { addAbortSignal } = require("node:stream");
const http = require("node:http");
const { once } = require("node:events");
const { S3Client, PutObjectCommand, GetObjectCommand } = require("@aws-sdk/client-s3");
const { DynamoDBClient, PutItemCommand, GetItemCommand } = require("@aws-sdk/client-dynamodb");
const versions = {
    s3: require("@aws-sdk/client-s3/package.json").version,
    dynamodb: require("@aws-sdk/client-dynamodb/package.json").version,
};
let counter = 0;

function remaining(deadline) {
    const value = Math.ceil(deadline - performance.now());
    assert(value > 0, "SDK fixture deadline expired");
    return value;
}
function selectorsCleared() {
    return ["AWS_PROFILE", "AWS_DEFAULT_PROFILE", "AWS_SESSION_TOKEN",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI", "AWS_ENDPOINT_URL_S3", "AWS_ENDPOINT_URL_DYNAMODB"]
        .every(key => process.env[key] === undefined);
}
async function bodyBytes(body, signal, deadline) {
    const chunks = [];
    let total = 0;
    try {
        addAbortSignal(signal, body);
        for await (const chunk of body) {
            remaining(deadline);
            signal.throwIfAborted();
            total += chunk.length;
            assert(total <= 4096, "SDK response body exceeds fixture cap");
            chunks.push(chunk);
        }
        remaining(deadline);
        return Buffer.concat(chunks, total);
    } finally {
        body.destroy();
    }
}
class SdkS3AuthorizationError extends Error {
    constructor() { super("actual SDK AccessDenied/403 " + versions.s3); this.name = "SdkS3AuthorizationError"; }
}
class SdkDynamoAuthorizationError extends Error {
    constructor() { super("actual SDK AccessDeniedException/400 " + versions.dynamodb); this.name = "SdkDynamoAuthorizationError"; }
}

exports.handler = async event => {
    assert(event && typeof event.bucket === "string" && /^[a-z0-9-]{3,63}$/.test(event.bucket));
    assert(typeof event.table === "string" && /^[A-Za-z0-9_.-]{3,128}$/.test(event.table));
    assert(typeof event.key === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(event.key));
    assert(["roundtrip", "deny-s3", "deny-dynamodb"].includes(event.mode));
    assert(Number.isSafeInteger(event.value) && Math.abs(event.value) <= 1000000);
    counter += 1;
    assert(selectorsCleared(), "image SDK selector survived authoritative unsets");
    assert.equal(process.env.AWS_EC2_METADATA_DISABLED, "true");
    assert.equal(process.env.AWS_IGNORE_CONFIGURED_ENDPOINT_URLS, "false");
    assert.equal(process.env.AWS_CONFIG_FILE, "/dev/null");
    assert.equal(process.env.AWS_SHARED_CREDENTIALS_FILE, "/dev/null");
    const deadline = performance.now() + 10000;
    const controller = new AbortController();
    const signal = controller.signal;
    const s3 = new S3Client({ maxAttempts: 1, forcePathStyle: true });
    let dynamodb;
    let timer;
    const send = async (client, command) => {
        remaining(deadline);
        const response = await client.send(command, { abortSignal: signal });
        remaining(deadline);
        return response;
    };
    try {
        dynamodb = new DynamoDBClient({ maxAttempts: 1 });
        timer = setTimeout(() => controller.abort(new Error("SDK fixture deadline expired")), remaining(deadline));
        if (event.mode === "deny-s3") {
            try {
                const unexpected = await send(s3, new GetObjectCommand({ Bucket: event.bucket, Key: event.key }));
                unexpected.Body?.destroy();
            } catch (error) {
                assert.equal(error.name, "AccessDenied");
                assert.equal(error.$metadata.httpStatusCode, 403);
                throw new SdkS3AuthorizationError();
            }
            throw new Error("wrong explicit SDK credentials unexpectedly authorized S3");
        }
        if (event.mode === "deny-dynamodb") {
            try {
                await send(dynamodb, new GetItemCommand({ TableName: event.table, Key: { id: { S: event.key } } }));
            } catch (error) {
                assert.equal(error.name, "AccessDeniedException");
                assert.equal(error.$metadata.httpStatusCode, 400);
                throw new SdkDynamoAuthorizationError();
            }
            throw new Error("wrong explicit SDK credentials unexpectedly authorized DynamoDB");
        }
        const computed = event.value * 2;
        const expected = Buffer.from(`rustack-sdk:${event.value}:${computed}`);
        await send(s3, new PutObjectCommand({ Bucket: event.bucket, Key: event.key, Body: expected }));
        const object = await send(s3, new GetObjectCommand({ Bucket: event.bucket, Key: event.key }));
        assert.deepEqual(await bodyBytes(object.Body, signal, deadline), expected);
        await send(dynamodb, new PutItemCommand({ TableName: event.table,
            Item: { id: { S: event.key }, result: { N: String(computed) } } }));
        const item = await send(dynamodb, new GetItemCommand({ TableName: event.table,
            Key: { id: { S: event.key } }, ConsistentRead: true }));
        assert.equal(item.Item.result.N, String(computed));
        return { computed, counter, versions, selectorsCleared: true,
            nodeVersion: process.versions.node, architecture: process.arch };
    } finally {
        clearTimeout(timer);
        controller.abort();
        s3.destroy();
        dynamodb?.destroy();
    }
};


exports.bodyControl = async () => {
    const deadline = performance.now() + 2000;
    const controller = new AbortController();
    const signal = controller.signal;
    const budgetTimer = setTimeout(() => controller.abort(), remaining(deadline));
    let heldReply, request, response, abortTimer, headerAbort;
    let headersObserved = false;
    let rejected = false;
    const server = http.createServer((_request, reply) => {
        heldReply = reply;
        reply.writeHead(200, { "Content-Length": "1" });
        reply.flushHeaders();
    });
    try {
        server.listen(0, "127.0.0.1");
        await once(server, "listening", { signal });
        remaining(deadline);
        response = await new Promise((resolve, reject) => {
            signal.throwIfAborted();
            request = http.get({ host: "127.0.0.1", port: server.address().port, path: "/quiet", agent: false });
            headerAbort = () => request.destroy(new Error("control header deadline"));
            signal.addEventListener("abort", headerAbort, { once: true });
            request.once("error", reject);
            request.once("response", value => {
                signal.removeEventListener("abort", headerAbort);
                headersObserved = true;
                resolve(value);
            });
        });
        abortTimer = setTimeout(() => controller.abort(), 25);
        try {
            await bodyBytes(response, signal, deadline);
        } catch (error) {
            assert.equal(error.name, "AbortError");
            rejected = true;
        }
        assert(rejected && response.destroyed && headersObserved, "quiet SDK body did not abort");
    } finally {
        clearTimeout(budgetTimer);
        clearTimeout(abortTimer);
        if (headerAbort) signal.removeEventListener("abort", headerAbort);
        controller.abort();
        response?.destroy();
        request?.destroy();
        heldReply?.destroy();
        server.closeAllConnections();
        if (server.listening) await new Promise(resolve => server.close(resolve));
    }
    remaining(deadline);
    return { quietBodyRejected: rejected, headersObserved, destroyed: response.destroyed };
};
