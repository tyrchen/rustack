# Rustack Pulumi Hackathon App Validation Spec

**Type:** Validation Spec
**Status:** Implemented
**Scope:** Validate that Rustack can be used as a Pulumi AWS provider target for
a non-trivial serverless application topology rather than only independent
single-service resources.

## 1. Goal

Build and continuously validate a realistic hackathon application stack:

- Frontend: CloudFront -> S3.
- API: CloudFront -> API Gateway V2 -> Lambda -> DynamoDB / S3 / SQS / SSM.
- Protected data: CloudFront token function -> S3.
- Image processing: API Lambda -> S3 + SQS -> Lambda worker -> S3 + DynamoDB.

The validation target is provisioning compatibility: Pulumi must create, read,
export, destroy, and remove all resources through Rustack without unsupported
provider API calls or incompatible response shapes.

## 2. Pulumi Project

Path:

```text
examples/pulumi/hackathon-app
```

Entrypoint:

```text
examples/pulumi/hackathon-app/index.ts
```

The project uses the standard `@pulumi/aws` provider and points every relevant
AWS service endpoint to Rustack with the same provider contract documented in
`docs/pulumi.md`.

## 3. Provisioned Resources

The stack provisions:

- `aws.s3.Bucket` for the static frontend, protected data, and image uploads.
- `aws.s3.BucketObject` for frontend assets and protected seed data.
- `aws.s3.BucketPolicy` for CloudFront Origin Access Control reads.
- `aws.dynamodb.Table` with streams enabled for project/image state.
- `aws.sqs.Queue` for image work dispatch.
- `aws.ssm.Parameter` as a `SecureString` for the protected route token
  material.
- `aws.iam.Role` and `aws.iam.RolePolicy` for Lambda execution permissions.
- `aws.lambda.Function` for the API and image worker.
- `aws.lambda.EventSourceMapping` from SQS to the worker Lambda.
- `aws.lambda.Permission` for API Gateway invocation.
- `aws.apigatewayv2.Api`, `aws.apigatewayv2.Integration`,
  `aws.apigatewayv2.Route`, and `aws.apigatewayv2.Stage`.
- `aws.cloudfront.Function` for token checking on `/protected/*`.
- `aws.cloudfront.OriginAccessControl` for S3 origins.
- `aws.cloudfront.Distribution` with S3 frontend, protected S3, and API Gateway
  origins plus ordered cache behaviors.

## 4. Resource Graph

```text
viewer
  -> CloudFront distribution
     -> /index.html -> S3 site bucket
     -> /api/* -> API Gateway V2 -> Lambda API
          -> DynamoDB projects table
          -> S3 upload bucket
          -> SQS image queue
          -> SSM token parameter
     -> /protected/* -> CloudFront Function token check -> S3 protected bucket

Lambda API
  -> S3 upload bucket
  -> SQS image queue

SQS image queue
  -> Lambda event source mapping
  -> Lambda worker
     -> S3 upload bucket processed prefix
     -> DynamoDB projects table
```

## 5. Validation Command

```bash
RUSTACK_ENDPOINT=http://127.0.0.1:4567 PULUMI_STACK=rustack-hackathon-final make pulumi-hackathon-smoke
```

The smoke runner:

1. Starts Rustack on the requested local endpoint when no healthy server exists.
2. Installs Node dependencies.
3. Runs `npm run typecheck`.
4. Uses a temporary Pulumi file backend.
5. Runs `pulumi up --yes --skip-preview`.
6. Prints stack outputs as JSON.
7. Runs `pulumi destroy --yes --skip-preview`.
8. Removes the Pulumi stack, local stack config, temporary backend, and child
   Rustack process.

## 6. Success Criteria

The target passes only when:

- Pulumi creates all resources without provider initialization failures.
- Pulumi read-after-write checks succeed for each resource.
- Stack outputs include CloudFront distribution ID/domain, API endpoint/stage
  URL, Lambda ARNs, SQS URL, DynamoDB table/stream ARNs, S3 bucket names, and
  SSM parameter name.
- Pulumi destroy completes successfully and cleanup removes temporary state.

## 7. Verified Result

Validated on May 26, 2026 with:

```bash
RUSTACK_ENDPOINT=http://127.0.0.1:4567 PULUMI_STACK=rustack-hackathon-final make pulumi-hackathon-smoke
```

Result:

- `npm run typecheck` passed.
- Pulumi created 26 resources in 40 seconds.
- Outputs included API endpoint, CloudFront distribution ID/domain, Lambda ARNs,
  SQS URL, DynamoDB table/stream, S3 buckets, and SSM parameter.
- The SQS URL followed the configured Rustack endpoint:
  `http://127.0.0.1:4567/...`.
- Pulumi destroy and stack cleanup completed.
