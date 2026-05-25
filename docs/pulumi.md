# Pulumi

Rustack can be used as a Pulumi deployment target through the standard Pulumi
AWS provider. The provider stays `@pulumi/aws`; Rustack supplies the local AWS
API endpoints.

## Quick Start

```bash
make pulumi-smoke
```

The target runs `examples/pulumi/rustack-target`, which validates STS and
creates an SQS queue against Rustack.

## Provider Contract

Use one explicit AWS provider and pass it to every resource:

```ts
const rustack = new aws.Provider("rustack", {
  accessKey: "AKIAIOSFODNN7EXAMPLE",
  secretKey: pulumi.secret("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"),
  region: "us-east-1",
  endpoints: [{
    s3: "http://127.0.0.1:4566",
    sqs: "http://127.0.0.1:4566",
    sns: "http://127.0.0.1:4566",
    dynamodb: "http://127.0.0.1:4566",
    iam: "http://127.0.0.1:4566",
    sts: "http://127.0.0.1:4566",
  }],
  s3UsePathStyle: true,
  skipCredentialsValidation: true,
  skipMetadataApiCheck: true,
  skipRegionValidation: true,
  skipRequestingAccountId: true,
});
```

The Pulumi AWS provider performs some credential checks during provider
initialization, before user resources are created. The smoke example skips those
bootstrap checks and then calls `aws.getCallerIdentityOutput` through the
Rustack provider to verify STS endpoint routing explicitly.

## Current Scope

The checked smoke path covers Pulumi provider initialization, STS
`GetCallerIdentity`, and SQS queue CRUD. Add more services to
`examples/pulumi/rustack-target/index.ts` only after the matching Rustack service
supports the extra read-after-write calls made by the Pulumi AWS provider.
