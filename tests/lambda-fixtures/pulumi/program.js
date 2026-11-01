"use strict";

const { performance } = require("node:perf_hooks");
const pulumi = require("@pulumi/pulumi");
const aws = require("@pulumi/aws");
const support = require("./program_support");

module.exports = (async () => {
    const file = process.env.RUSTACK_IAC_CONFIG;
    if (typeof file !== "string" || !/^\/tmp\/iac-[0-9a-f-]{36}\/config.json$/.test(file)) {
        throw new Error("invalid IaC configuration path");
    }
    const config = support.validate(await support.smallJSON(file, 8192));
    const key = process.env.AWS_ACCESS_KEY_ID, secret = process.env.AWS_SECRET_ACCESS_KEY;
    if (typeof key !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(key) ||
        typeof secret !== "string" || !/^[\x21-\x7e]{1,256}$/.test(secret)) {
        throw new Error("missing explicit IaC fixture credentials");
    }
    const provider = new aws.Provider("local", {
        accessKey: pulumi.secret(key), secretKey: pulumi.secret(secret), region: "us-east-1",
        endpoints: [{ sfn: config.endpoint }], maxRetries: 0,
        skipCredentialsValidation: true, skipMetadataApiCheck: true, skipRegionValidation: true,
        skipRequestingAccountId: true, sharedConfigFiles: [config.emptyConfig],
        sharedCredentialsFiles: [config.emptyConfig],
    }, { version: "7.52.1" });
    const value = Number(process.env.RUSTACK_IAC_VALUE);
    const args = support.resourceArgs(config, value, process.env.RUSTACK_IAC_UNSUPPORTED);
    const machine = new aws.sfn.StateMachine("target", args, { provider, version: "7.52.1" });
    const ordinal = Number(process.env.RUSTACK_IAC_COMMAND);
    const stage = process.env.RUSTACK_IAC_GATE;
    return { targetArn: machine.arn.apply(arn => support.gateResource(arn, config, {
        dryRun: pulumi.runtime.isDryRun(), stage, ordinal,
        workspace: file.slice(0, -"/config.json".length), deadline: performance.now() + 60000,
    })), value };
})();
