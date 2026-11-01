"use strict";

const { createHash, timingSafeEqual } = require("node:crypto");
const { performance } = require("node:perf_hooks");
const { setTimeout: delay } = require("node:timers/promises");
const { LambdaClient, InvokeCommand } = require("@aws-sdk/client-lambda");
const { DynamoDBClient, PutItemCommand, GetItemCommand } = require("@aws-sdk/client-dynamodb");
const { SFNClient, SendTaskSuccessCommand, SendTaskFailureCommand,
    SendTaskHeartbeatCommand, GetExecutionHistoryCommand } = require("@aws-sdk/client-sfn");

const versions = {
    lambda: require("@aws-sdk/client-lambda/package.json").version,
    sfn: require("@aws-sdk/client-sfn/package.json").version,
    dynamodb: require("@aws-sdk/client-dynamodb/package.json").version,
};
const modes = ["early", "late", "externalFailure", "invocationFailure", "retry",
    "heartbeat", "absoluteTimeout", "heartbeatTimeout", "stop"];
const hex = /^[0-9a-f]{64}$/;
let counter = 0;
let previousToken;

class CallbackFixtureFailure extends Error {
    constructor() { super("bounded image callback fixture failed"); this.name = "CallbackFixtureFailure"; }
}
class CallbackInvocationFailure extends Error {
    constructor() { super("actual image invocation failure"); this.name = "CallbackInvocationFailure"; }
}
function check(value) {
    if (!value) throw new CallbackFixtureFailure();
}
function sameToken(left, right) {
    check(hex.test(left) && hex.test(right));
    const a = Buffer.from(left, "ascii"), b = Buffer.from(right, "ascii");
    try { return timingSafeEqual(a, b); } finally { a.fill(0); b.fill(0); }
}
function input(event) {
    check(event && typeof event === "object" && !Array.isArray(event));
    check(Object.keys(event).length === 8 && Object.keys(event).every(key =>
        ["token", "executionArn", "nonce", "table", "worker", "n", "retry", "mode"].includes(key)));
    check(typeof event.token === "string" && hex.test(event.token));
    check(typeof event.executionArn === "string" && event.executionArn.length <= 256 &&
        /^arn:aws:states:us-east-1:000000000000:execution:[A-Za-z0-9_-]+:[A-Za-z0-9_-]+$/.test(event.executionArn));
    check(typeof event.nonce === "string" && /^[0-9a-f]{32}$/.test(event.nonce));
    check(typeof event.table === "string" && /^[A-Za-z0-9_.-]{3,128}$/.test(event.table));
    check(typeof event.worker === "string" && /^[A-Za-z0-9_-]{1,64}$/.test(event.worker));
    check(Number.isSafeInteger(event.n) && Math.abs(event.n) <= 1000000);
    check(Number.isInteger(event.retry) && event.retry >= 0 && event.retry <= 1 && modes.includes(event.mode));
    return event;
}
function checkEnvironment() {
    for (const key of ["AWS_PROFILE", "AWS_ENDPOINT_URL_LAMBDA", "AWS_ENDPOINT_URL_SFN",
        "AWS_ENDPOINT_URL_DYNAMODB"]) check(process.env[key] === undefined);
    check(process.env.AWS_IGNORE_CONFIGURED_ENDPOINT_URLS === "false");
    check(process.env.AWS_EC2_METADATA_DISABLED === "true");
}
function remaining(deadline) {
    const value = Math.ceil(deadline - performance.now());
    check(value > 0);
    return value;
}
function apiError(error, name) {
    return error?.name === name && error?.$metadata?.httpStatusCode === 400;
}

class Session {
    constructor(deadline, signal) {
        this.deadline = deadline;
        this.signal = signal;
        this.clients = [];
    }
    open() {
        this.sfn = this.add(new SFNClient({ maxAttempts: 1 }));
        this.ddb = this.add(new DynamoDBClient({ maxAttempts: 1 }));
        this.lambda = this.add(new LambdaClient({ maxAttempts: 1 }));
    }
    add(client) { this.clients.push(client); return client; }
    close() { for (const client of this.clients) client.destroy(); }
    async send(client, command) {
        this.signal.throwIfAborted();
        const request = new AbortController();
        const forward = () => request.abort();
        const timer = setTimeout(() => request.abort(), Math.min(4000, remaining(this.deadline)));
        this.signal.addEventListener("abort", forward, { once: true });
        try {
            this.signal.throwIfAborted();
            remaining(this.deadline);
            const value = await client.send(command, { abortSignal: request.signal });
            this.signal.throwIfAborted();
            remaining(this.deadline);
            return value;
        } finally {
            clearTimeout(timer);
            this.signal.removeEventListener("abort", forward);
            request.abort();
        }
    }
    async expect(command, name) {
        try { await this.send(this.sfn, command); } catch (error) {
            check(apiError(error, name));
            return;
        }
        throw new CallbackFixtureFailure();
    }
    async gate(data, kind) {
        const result = await this.send(this.ddb, new GetItemCommand({
            TableName: data.table, Key: { id: { S: "gate:" + data.nonce + ":" + data.retry + ":" + kind } }, ConsistentRead: true,
        }));
        return result.Item?.open?.BOOL === true;
    }
    async waitGate(data, kind) {
        while (!await this.gate(data, kind)) {
            await delay(Math.min(100, remaining(this.deadline)), undefined, { signal: this.signal });
        }
    }
    async record(data, state, stage) {
        const item = {
            id: { S: state.requestId }, nonce: { S: data.nonce }, role: { S: state.role },
            requestId: { S: state.requestId }, tokenHash: { S: state.tokenHash }, stage: { S: stage },
            retry: { N: String(data.retry) }, heartbeats: { N: String(state.heartbeats) },
            historySafe: { BOOL: state.historySafe }, closedToken: { BOOL: state.closedToken },
            oldTokenRejected: { BOOL: state.oldTokenRejected }, sdkVersion: { S: versions.sfn },
        };
        check(Buffer.byteLength(JSON.stringify(item)) <= 2048);
        await this.send(this.ddb, new PutItemCommand({ TableName: data.table, Item: item }));
    }
    async history(data, state) {
        const result = await this.send(this.sfn, new GetExecutionHistoryCommand({
            executionArn: data.executionArn, includeExecutionData: true, maxResults: 1000,
        }));
        check(!result.nextToken && Array.isArray(result.events) && result.events.length <= 256);
        const encoded = JSON.stringify(result);
        check(Buffer.byteLength(encoded) <= 262144 && !encoded.includes(data.token));
        check(result.events.some(event => event.type === "TaskScheduled"));
        state.historySafe = true;
    }
    async callback(data, state, failure) {
        const forged = "0".repeat(64);
        check(!sameToken(data.token, forged));
        await this.expect(new SendTaskSuccessCommand({ taskToken: forged, output: "{}" }), "InvalidToken");
        const command = failure
            ? new SendTaskFailureCommand({ taskToken: data.token, error: failure, cause: "actual image SDK failure" })
            : new SendTaskSuccessCommand({ taskToken: data.token, output: JSON.stringify({
                value: data.n * 2, sdk: "node", requestId: state.requestId, retry: data.retry,
            }) });
        const result = await this.send(this.sfn, command);
        check(result.$metadata.httpStatusCode === 200);
        await this.expect(new SendTaskHeartbeatCommand({ taskToken: data.token }), "TaskTimedOut");
        state.closedToken = true;
        await this.history(data, state);
    }
}

async function dispatcher(session, data, state) {
    if (data.retry === 1) {
        check(previousToken !== undefined && !sameToken(previousToken, data.token));
        await session.expect(new SendTaskHeartbeatCommand({ taskToken: previousToken }), "TaskTimedOut");
        state.oldTokenRejected = true;
    }
    previousToken = data.token;
    await session.record(data, state, "consumed");
    if (["early", "externalFailure", "invocationFailure", "retry"].includes(data.mode)) {
        const failure = data.mode === "externalFailure" ? "CallbackExternalFailure"
            : data.mode === "retry" && data.retry === 0 ? "CallbackRetry" : undefined;
        await session.callback(data, state, failure);
        await session.record(data, state, "callback");
        await session.waitGate(data, "return");
        if (data.mode === "invocationFailure") throw new CallbackInvocationFailure();
    } else {
        if (data.mode === "heartbeat") {
            const beat = await session.send(session.sfn, new SendTaskHeartbeatCommand({ taskToken: data.token }));
            check(beat.$metadata.httpStatusCode === 200);
        }
        const body = Buffer.from(JSON.stringify(data));
        check(body.length <= 4096);
        const accepted = await session.send(session.lambda, new InvokeCommand({
            FunctionName: data.worker, InvocationType: "Event", Payload: body,
        }));
        check(accepted.StatusCode === 202 && accepted.$metadata.httpStatusCode === 202);
        check(!accepted.FunctionError && (!accepted.Payload || accepted.Payload.length === 0));
        await session.record(data, state, "accepted");
        if (["heartbeat", "stop"].includes(data.mode)) await session.waitGate(data, "return");
    }
    return { ignored: "synchronous callback body", requestId: state.requestId };
}

async function worker(session, data, state) {
    check(["late", "heartbeat", "absoluteTimeout", "heartbeatTimeout", "stop"].includes(data.mode));
    await session.record(data, state, "waiting");
    while (!await session.gate(data, "finish")) {
        if (["heartbeat", "absoluteTimeout"].includes(data.mode) && !state.closedToken &&
            await session.gate(data, "heartbeats")) {
            try {
                const result = await session.send(session.sfn, new SendTaskHeartbeatCommand({ taskToken: data.token }));
                check(result.$metadata.httpStatusCode === 200);
                state.heartbeats += 1;
                check(state.heartbeats <= 100);
            } catch (error) {
                check(data.mode === "absoluteTimeout" && apiError(error, "TaskTimedOut"));
                state.closedToken = true;
            }
            await session.record(data, state, "heartbeat");
        }
        await delay(Math.min(100, remaining(session.deadline)), undefined, { signal: session.signal });
    }
    if (["absoluteTimeout", "heartbeatTimeout", "stop"].includes(data.mode)) {
        await session.expect(new SendTaskSuccessCommand({ taskToken: data.token, output: "{}" }), "TaskTimedOut");
        state.closedToken = true;
        await session.history(data, state);
    } else {
        await session.callback(data, state);
    }
    await session.record(data, state, "finished");
    return { workerFinished: true, requestId: state.requestId };
}

async function run(event, context, isWorker) {
    let session, timer;
    const controller = new AbortController();
    try {
        checkEnvironment();
        counter += 1;
        check(counter <= 128 && /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(context.awsRequestId));
        if (event?.mode === "probe" || event?.mode === "compute") {
            check(Object.keys(event).every(key => ["mode", "n"].includes(key)));
            if (event.mode === "compute") check(Number.isSafeInteger(event.n) && Math.abs(event.n) <= 1000000);
            return { probe: true, computed: event.mode === "compute" ? event.n * 2 : 0,
                counter, versions, requestId: context.awsRequestId, architecture: process.arch };
        }
        const data = input(event);
        const time = context.getRemainingTimeInMillis();
        check(Number.isFinite(time) && time > 100);
        const deadline = performance.now() + Math.min(35000, time - 100);
        timer = setTimeout(() => controller.abort(), remaining(deadline));
        session = new Session(deadline, controller.signal);
        session.open();
        const state = { role: isWorker ? "B" : "A", requestId: context.awsRequestId,
            tokenHash: createHash("sha256").update(data.token, "ascii").digest("hex"),
            heartbeats: 0, historySafe: false, closedToken: false, oldTokenRejected: false };
        return await (isWorker ? worker(session, data, state) : dispatcher(session, data, state));
    } catch (error) {
        if (error instanceof CallbackInvocationFailure) throw error;
        throw new CallbackFixtureFailure();
    } finally {
        clearTimeout(timer);
        controller.abort();
        session?.close();
    }
}

exports.dispatcher = (event, context) => run(event, context, false);
exports.worker = (event, context) => run(event, context, true);
