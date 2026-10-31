"use strict";

const { performance } = require("node:perf_hooks");
const runtime = require("./pulumi/iac_runtime");
const { input } = require("./pulumi/iac_input");
let counter = 0;

class IacFixtureFailure extends Error {
    constructor(stage) { super("bounded managed IaC fixture failed at " + stage); this.name = "IacFixtureFailure"; }
}

exports.handler = async (event, context) => {
    let session, stage = "validation";
    try {
        counter += 1;
        runtime.check(counter <= 16 && typeof context.getRemainingTimeInMillis === "function");
        const remaining = context.getRemainingTimeInMillis();
        runtime.check(Number.isFinite(remaining) && remaining > 1000);
        const deadline = performance.now() + Math.min(700000, remaining - 1000);
        const probe = event && typeof event === "object" && !Array.isArray(event) && event.mode === "probe";
        let data;
        if (probe) {
            runtime.check(Object.keys(event).length === 1 && typeof context.awsRequestId === "string" &&
                /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(context.awsRequestId));
        } else data = input(event, context.awsRequestId);
        stage = "provenance";
        const versions = await runtime.source(deadline);
        stage = "versions";
        runtime.check(versions.cli === "3.273.0" && versions.sdk === "3.273.0" && versions.aws === "7.52.1" &&
            versions.provider === "7.52.1" && versions.dynamodb === "3.1105.0" && versions.node === "24.22.1");
        if (probe) {
            return { probe: true, requestId: context.awsRequestId, versions, counter, architecture: process.arch };
        }
        const { Session } = require("./pulumi/iac_session");
        session = new Session(data, deadline, versions);
        stage = "setup";
        await session.setup();
        stage = "lifecycle";
        return await session.lifecycle();
    } catch {
        // Function errors are physically retired by InstanceExecutor before the
        // observed invocation completes, including separately grouped plugins.
        throw new IacFixtureFailure(stage);
    } finally { session?.client.destroy(); }
};
