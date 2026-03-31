# Rustack IAM: Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-sns-design.md](./rustack-sns-design.md)
**Scope:** Add AWS IAM (Identity and Access Management) support to Rustack -- ~60 operations across 4 phases covering users, roles, groups, policies, instance profiles, access keys, tagging, and service-linked roles, using the same Smithy-based codegen and gateway routing patterns established by SNS (awsQuery protocol).

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: awsQuery](#5-protocol-design-awsquery)
6. [Smithy Code Generation Strategy](#6-smithy-code-generation-strategy)
7. [Crate Structure](#7-crate-structure)
8. [HTTP Layer Design](#8-http-layer-design)
9. [Storage Engine Design](#9-storage-engine-design)
10. [Core Business Logic](#10-core-business-logic)
11. [Error Handling](#11-error-handling)
12. [Server Integration](#12-server-integration)
13. [Testing Strategy](#13-testing-strategy)
14. [Phased Implementation Plan](#14-phased-implementation-plan)
15. [Risk Analysis](#15-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding AWS IAM support to Rustack. Key points:

- **Large scope** -- ~60 operations across 4 phases, making this one of the larger services in Rustack. However, the vast majority of operations are straightforward CRUD with no complex state machines, streaming, or background processing.
- **High value** -- IAM is the foundational AWS service. Every Terraform plan, CDK deployment, and CI pipeline that creates roles, policies, or instance profiles depends on IAM. Without local IAM, developers must either skip IAM resources in their IaC or make real AWS API calls. Adding IAM unlocks fully offline Terraform/CDK workflows for the most common AWS resources.
- **Global service** -- Unlike all other Rustack services, IAM is a **global** service (not regional). IAM entities are keyed by `account_id` only, not by `(account_id, region)`. This is a straightforward simplification of the storage model.
- **awsQuery protocol** -- IAM uses the `awsQuery` protocol (same as SNS). Requests are `POST` with `application/x-www-form-urlencoded` body containing `Action=OperationName`. Responses are XML. The entire form-parsing, XML-response, and error-formatting infrastructure from SNS can be reused.
- **Gateway routing challenge** -- Both IAM and SNS use `awsQuery` with `application/x-www-form-urlencoded`. The gateway must distinguish between them. The solution is to inspect the `Authorization` header's SigV4 credential scope, which contains the service name (`iam` vs `sns`). This is reliable because all AWS SDKs sign requests with the correct service name. As a fallback, IAM and SNS have completely disjoint `Action` parameter names.
- **Smithy codegen reuse** -- Generate a `rustack-iam-model` crate from the official Smithy model using the same codegen infrastructure as all other services.
- **Entity relationship model** -- IAM has a rich entity model: Users, Groups, Roles, Policies (managed and inline), Instance Profiles, and Access Keys. These entities have many-to-many relationships (e.g., policies attached to multiple users/roles/groups). The storage engine must efficiently track these relationships.
- **Estimated effort** -- 4-5 days for Phase 0 (~20 core operations), 8-10 days for full implementation (~60 operations), plus 1 day for CI integration.

---

## 2. Motivation

### 2.1 Why IAM?

AWS IAM is the identity and access control backbone of every AWS deployment. It is the single most-referenced AWS service in Infrastructure-as-Code:

- **Terraform** -- `aws_iam_role`, `aws_iam_policy`, `aws_iam_role_policy_attachment`, `aws_iam_instance_profile` are among the top 10 most-used Terraform AWS resources. Virtually every Terraform module that provisions compute, Lambda, ECS, or EKS resources requires IAM roles and policies.
- **CDK/CloudFormation** -- every `Function`, `StateMachine`, `TaskDefinition`, and `Cluster` construct generates IAM roles and policy attachments behind the scenes.
- **CI/CD pipelines** -- GitHub Actions, CircleCI, and Jenkins pipelines that run `terraform plan` or `cdk synth` against a local emulator need IAM to resolve role ARNs and policy documents.
- **Kubernetes (IRSA)** -- IAM Roles for Service Accounts (IRSA) creates OIDC providers and role trust policies. Testing IRSA configuration locally requires IAM.
- **Access key management** -- applications that manage their own credentials (CreateAccessKey, ListAccessKeys) need IAM for integration tests.
- **LocalStack dependency** -- many LocalStack users report that IAM is a prerequisite for testing any multi-service workflow, because services like Lambda, ECS, and Step Functions require execution roles.

Without local IAM, developers must either:
1. Hard-code ARNs and skip IAM resource creation in local dev
2. Use real AWS IAM (slow, requires internet, costs money at scale)
3. Mock IAM at the application level (fragile, incomplete)

### 2.2 Complexity Assessment

| Dimension               | IAM                                                       | SNS                                  | Secrets Manager                       | SSM              |
| ----------------------- | --------------------------------------------------------- | ------------------------------------ | ------------------------------------- | ---------------- |
| Total operations        | ~60                                                       | 42                                   | 23                                    | 13               |
| Complex state machines  | 0                                                         | 0                                    | 1 (rotation)                          | 0                |
| Entity types            | 6 (User, Group, Role, Policy, InstanceProfile, AccessKey) | 3 (Topic, Subscription, PlatformApp) | 1 (Secret with versions)              | 1 (Parameter)    |
| Relationship model      | Many-to-many (policy attachments)                         | One-to-many (topic->subscriptions)   | One-to-many (secret->versions)        | Flat             |
| Storage complexity      | 6 DashMaps + relationship tracking                        | 3 DashMaps                           | 1 DashMap + version map               | 1 DashMap        |
| Concurrency model       | Request/response only                                     | Request/response + delivery          | Request/response + deletion scheduler | Request/response |
| Protocol                | awsQuery (reuse SNS)                                      | awsQuery                             | awsJson1.1                            | awsJson1.1       |
| Global vs Regional      | Global (simpler)                                          | Regional                             | Regional                              | Regional         |
| Estimated lines of code | ~6,000                                                    | ~8,000                               | ~4,500                                | ~3,000           |

IAM has more operations than any other Rustack service, but the individual operations are simpler -- almost all are CRUD without complex state transitions. The primary complexity is the entity relationship model and ensuring correct ARN generation.

### 2.3 Tool Coverage

With all ~60 operations implemented, the following tools work out of the box:

| Tool                                               | Operations Used                                                                                   | Phase Available   |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ----------------- |
| AWS CLI (`aws iam`)                                | All CRUD ops                                                                                      | Phase 0+          |
| Terraform (`aws_iam_role`, `aws_iam_policy`, etc.) | CreateRole, CreatePolicy, AttachRolePolicy, CreateInstanceProfile, AddRoleToInstanceProfile, tags | Phase 0 + Phase 1 |
| AWS CDK                                            | CreateRole, CreatePolicy, AttachRolePolicy, PutRolePolicy                                         | Phase 0 + Phase 2 |
| Serverless Framework                               | CreateRole, AttachRolePolicy, PutRolePolicy                                                       | Phase 0 + Phase 2 |
| Pulumi                                             | CreateRole, CreatePolicy, AttachRolePolicy                                                        | Phase 0           |
| eksctl                                             | CreateRole, CreatePolicy, AttachRolePolicy, CreateInstanceProfile                                 | Phase 0 + Phase 1 |
| Kubernetes IRSA                                    | CreateRole (with trust policy), AttachRolePolicy                                                  | Phase 0           |
| GitHub Actions (OIDC)                              | CreateRole, GetRole, AttachRolePolicy                                                             | Phase 0           |
| Ansible (amazon.aws collection)                    | All user/role/policy CRUD                                                                         | Phase 0+          |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full core API** -- implement ~60 IAM operations across 4 phases covering users, groups, roles, managed policies, inline policies, instance profiles, access keys, tagging, and service-linked roles
2. **Correct ARN generation** -- generate valid ARNs for all entity types (`arn:aws:iam::{account_id}:user/{path}{name}`, `arn:aws:iam::{account_id}:role/{path}{name}`, etc.)
3. **Policy document storage** -- accept, store, and return IAM policy documents as JSON strings. Validate JSON syntax but do not parse or enforce the policy language.
4. **Managed policy versioning** -- support `CreatePolicyVersion`, `ListPolicyVersions`, `SetDefaultPolicyVersion`, `DeletePolicyVersion` with up to 5 versions per policy
5. **Inline policy CRUD** -- support `PutUserPolicy`/`PutRolePolicy`/`PutGroupPolicy` and corresponding Get/Delete/List operations
6. **Policy attachment tracking** -- track which managed policies are attached to which users, roles, and groups. Support list operations in both directions (policies on an entity, entities on a policy).
7. **Access key management** -- `CreateAccessKey`, `DeleteAccessKey`, `ListAccessKeys`, `UpdateAccessKey` (enable/disable), `GetAccessKeyLastUsed`
8. **Instance profile management** -- `CreateInstanceProfile`, `DeleteInstanceProfile`, `ListInstanceProfiles`, `AddRoleToInstanceProfile`, `RemoveRoleFromInstanceProfile`
9. **Tag support** -- `TagUser`, `UntagUser`, `TagRole`, `UntagRole`, `ListUserTags`, `ListRoleTags`
10. **Service-linked roles** -- `CreateServiceLinkedRole`, `DeleteServiceLinkedRole`, `GetServiceLinkedRoleDeletionStatus`
11. **Assume role policy management** -- `UpdateAssumeRolePolicy` to update role trust policies
12. **Smithy-generated types** -- all types generated from official AWS Smithy model
13. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsQuery protocol layer from SNS
14. **Same Docker image** -- single binary serves all services on port 4566
15. **Pass LocalStack and moto IAM test suites** -- validate against comprehensive mock test suites

### 3.2 Non-Goals

1. **Policy enforcement** -- IAM policies are stored but never evaluated for authorization decisions. All API calls to all services succeed regardless of IAM policies. This matches LocalStack's behavior.
2. **STS operations** -- `AssumeRole`, `AssumeRoleWithSAML`, `AssumeRoleWithWebIdentity`, `GetCallerIdentity`, `GetSessionToken`, and `GetFederationToken` are STS operations (service endpoint `sts.amazonaws.com`), not IAM. STS will be a separate service. `GetCallerIdentity` is noted in the user's request but belongs to STS.
3. **OIDC/SAML providers** -- `CreateOpenIDConnectProvider`, `CreateSAMLProvider`, and related operations are deferred to a future phase. They can be added as stubs that store configuration without validation.
4. **Account password policy** -- `GetAccountPasswordPolicy`, `UpdateAccountPasswordPolicy`, `DeleteAccountPasswordPolicy` are low-priority stubs.
5. **MFA device management** -- `CreateVirtualMFADevice`, `EnableMFADevice`, `DeactivateMFADevice`, `ListMFADevices` are low priority for local dev.
6. **Login profiles** -- `CreateLoginProfile`, `GetLoginProfile`, `UpdateLoginProfile`, `DeleteLoginProfile` manage console passwords, irrelevant for API-based local testing.
7. **SSH public keys** -- `UploadSSHPublicKey`, `ListSSHPublicKeys`, etc. are low priority.
8. **Server certificates** -- `UploadServerCertificate`, `ListServerCertificates`, etc. are low priority.
9. **Account aliases** -- `CreateAccountAlias`, `ListAccountAliases`, `DeleteAccountAlias` are low priority.
10. **Credential reports** -- `GenerateCredentialReport`, `GetCredentialReport` are low priority.
11. **Service-specific credentials** -- `CreateServiceSpecificCredential`, `ListServiceSpecificCredentials`, etc. are low priority.
12. **Organizations integration** -- no cross-account policy evaluation or SCPs.
13. **Data persistence across restarts** -- in-memory only, matching all other Rustack services.
14. **Policy simulation engine** -- `SimulatePrincipalPolicy` and `SimulateCustomPolicy` accept requests and return stub results indicating all actions are allowed. No actual policy evaluation engine is built.

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                AWS SDK / CLI / Terraform / CDK
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  SigV4 service name + Action= dispatch
              +--------+------------+
                       |
     +------+------+------+------+------+------+
     |      |      |      |      |      |
     v      v      v      v      v      v
+------+ +-----+ +-----+ +-----+ +-----+ +-----+
| S3   | | DDB | | SQS | | SSM | | SNS | | IAM |
|(Xml) | |(J10)| |(Qry)| |(J11)| |(Qry)| |(Qry)|
+--+---+ +--+--+ +--+--+ +--+--+ +--+--+ +--+--+
   |        |        |       |       |       |
+--+---+ +--+--+ +--+--+ +--+--+ +--+--+ +--+--+
|S3    | |DDB  | |SQS  | |SSM  | |SNS  | |IAM  |
|Core  | |Core | |Core | |Core | |Core | |Core |
+--+---+ +--+--+ +--+--+ +--+--+ +--+--+ +--+--+
   |        |        |       |       |       |
   +--------+--------+-------+-------+-------+
                      |
               +------+------+
               | rustack-  |
               | core + auth |
               +-------------+
```

### 4.2 Gateway Routing: Distinguishing IAM from SNS

Both IAM and SNS use the `awsQuery` protocol with `application/x-www-form-urlencoded` Content-Type and `POST /` requests. The gateway must distinguish between them. There are two complementary strategies:

**Strategy 1: SigV4 credential scope (primary)**

Every AWS SDK signs requests with SigV4, and the `Authorization` header contains the credential scope which includes the service name:

```
Authorization: AWS4-HMAC-SHA256
  Credential=AKIAIOSFODNN7EXAMPLE/20260319/us-east-1/iam/aws4_request,
  SignedHeaders=content-type;host;x-amz-date,
  Signature=...
```

The fourth component of the credential scope is the service name: `iam` for IAM, `sns` for SNS. The gateway can extract this without buffering or parsing the body.

```rust
/// Extract the SigV4 service name from the Authorization header.
fn extract_sigv4_service(req: &http::Request<Incoming>) -> Option<&str> {
    let auth = req.headers().get("authorization")?.to_str().ok()?;
    // Format: AWS4-HMAC-SHA256 Credential=AKID/date/region/SERVICE/aws4_request, ...
    let cred_start = auth.find("Credential=")? + "Credential=".len();
    let cred_end = auth[cred_start..].find(',').map(|i| cred_start + i)?;
    let credential = &auth[cred_start..cred_end];
    // credential = "AKID/date/region/SERVICE/aws4_request"
    let parts: Vec<&str> = credential.split('/').collect();
    // parts = ["AKID", "date", "region", "SERVICE", "aws4_request"]
    parts.get(3).copied()
}
```

**Strategy 2: Disjoint Action names (fallback)**

IAM and SNS have completely disjoint sets of `Action` parameter names. There is no overlap:
- IAM: `CreateUser`, `CreateRole`, `CreatePolicy`, `AttachRolePolicy`, `CreateInstanceProfile`, etc.
- SNS: `CreateTopic`, `Subscribe`, `Publish`, `PublishBatch`, etc.

If SigV4 parsing fails (e.g., unsigned requests in permissive mode), the gateway can fall back to inspecting the `Action` parameter, similar to how SNS currently works.

**Routing evaluation order:**

1. If `X-Amz-Target` starts with a known prefix -- route to the corresponding JSON-protocol service (DynamoDB, SQS-JSON, SSM, Secrets Manager, EventBridge, CloudWatch Logs, KMS, Kinesis)
2. If request is `POST /` with `Content-Type: application/x-www-form-urlencoded`:
   a. Extract SigV4 service name from `Authorization` header
   b. If service is `iam` -- route to IAM
   c. If service is `sns` -- route to SNS
   d. If no `Authorization` header or unparsable, inspect `Action=` parameter:
      - If Action is in `IAM_ACTIONS` set -- route to IAM
      - If Action is in `SNS_ACTIONS` set -- route to SNS
      - Otherwise, fall through
3. If request matches Lambda path pattern -- route to Lambda
4. Default: route to S3

| Service         | Protocol     | Content-Type                            | Dispatch Mechanism                             |
| --------------- | ------------ | --------------------------------------- | ---------------------------------------------- |
| DynamoDB        | awsJson1.0   | `application/x-amz-json-1.0`            | `X-Amz-Target: DynamoDB_20120810.*`            |
| SQS             | awsJson1.0   | `application/x-amz-json-1.0`            | `X-Amz-Target: AmazonSQS.*`                    |
| SSM             | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: AmazonSSM.*`                    |
| Secrets Manager | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: secretsmanager.*`               |
| EventBridge     | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: AWSEvents.*`                    |
| CloudWatch Logs | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: Logs_20140328.*`                |
| KMS             | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: TrentService.*`                 |
| Kinesis         | awsJson1.1   | `application/x-amz-json-1.1`            | `X-Amz-Target: Kinesis_20131202.*`             |
| SNS             | awsQuery     | `application/x-www-form-urlencoded`     | SigV4 service=`sns` or `Action` in SNS set     |
| **IAM**         | **awsQuery** | **`application/x-www-form-urlencoded`** | **SigV4 service=`iam` or `Action` in IAM set** |
| Lambda          | restJson1    | varies                                  | Path pattern `/YYYY-MM-DD/functions/*`         |
| S3              | restXml      | varies                                  | Catch-all (default)                            |

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-sns-{model,core,http}
+-- rustack-iam-model            <-- NEW (auto-generated)
+-- rustack-iam-core             <-- NEW
+-- rustack-iam-http             <-- NEW
+-- ... (other services)

rustack-iam-http
+-- rustack-iam-model
+-- rustack-auth
+-- quick-xml (XML response serialization, reuse pattern from SNS)
+-- serde_urlencoded (form request deserialization)

rustack-iam-core
+-- rustack-core
+-- rustack-iam-model
+-- dashmap
+-- serde_json (policy document validation)
+-- rand (access key ID generation)

rustack-iam-model (auto-generated, standalone)
+-- serde
+-- serde_json
```

---

## 5. Protocol Design: awsQuery

### 5.1 Protocol Comparison

IAM uses `awsQuery`, identical to SNS in wire format. The only differences are the XML namespace, API version, and SigV4 service name.

| Aspect                  | SNS (awsQuery)                             | IAM (awsQuery)                              |
| ----------------------- | ------------------------------------------ | ------------------------------------------- |
| HTTP Method             | POST only                                  | POST only                                   |
| URL Path                | `/` always                                 | `/` always                                  |
| Content-Type (request)  | `application/x-www-form-urlencoded`        | `application/x-www-form-urlencoded`         |
| Content-Type (response) | `text/xml`                                 | `text/xml`                                  |
| Operation dispatch      | `Action=<Op>` form parameter               | `Action=<Op>` form parameter                |
| Request body            | URL-encoded form fields                    | URL-encoded form fields                     |
| Response body           | XML                                        | XML                                         |
| Error body              | XML `<ErrorResponse>`                      | XML `<ErrorResponse>`                       |
| API version             | `2010-03-31`                               | `2010-05-08`                                |
| XML namespace           | `http://sns.amazonaws.com/doc/2010-03-31/` | `https://iam.amazonaws.com/doc/2010-05-08/` |
| Auth                    | SigV4, service=`sns`                       | SigV4, service=`iam`                        |
| Timestamps              | N/A                                        | ISO 8601 (`2026-03-19T12:00:00Z`)           |

### 5.2 What We Reuse from SNS

The SNS implementation provides all the awsQuery infrastructure IAM needs:

| Component                                   | Reusable?     | Notes                                                                                                                                                                 |
| ------------------------------------------- | ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Form parameter parsing (`serde_urlencoded`) | Yes           | Same `Action=<Op>&Param1=val1` parsing                                                                                                                                |
| XML response serialization (`XmlWriter`)    | Pattern reuse | IAM needs its own `XmlWriter` instance with different namespace, but the `XmlWriter` utility from SNS can be extracted into a shared crate or duplicated (small code) |
| XML error formatting (`<ErrorResponse>`)    | Pattern reuse | Same structure, different namespace URI                                                                                                                               |
| `ServiceRouter` trait                       | Yes           | IAM implements `ServiceRouter` like all other services                                                                                                                |
| SigV4 auth                                  | Yes           | `rustack-auth` is service-agnostic                                                                                                                                  |
| Form parameter encoding conventions         | Yes           | Same `member.N`, `entry.N.key/value` patterns for lists and maps                                                                                                      |

### 5.3 Wire Format Examples

**CreateRole request:**

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/iam/aws4_request, ...

Action=CreateRole
&RoleName=MyLambdaRole
&AssumeRolePolicyDocument=%7B%22Version%22%3A%222012-10-17%22%2C%22Statement%22%3A%5B%7B%22Effect%22%3A%22Allow%22%2C%22Principal%22%3A%7B%22Service%22%3A%22lambda.amazonaws.com%22%7D%2C%22Action%22%3A%22sts%3AAssumeRole%22%7D%5D%7D
&Path=%2F
&Version=2010-05-08
```

**CreateRole response:**

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<CreateRoleResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
  <CreateRoleResult>
    <Role>
      <Path>/</Path>
      <RoleName>MyLambdaRole</RoleName>
      <RoleId>AROAEXAMPLEID</RoleId>
      <Arn>arn:aws:iam::000000000000:role/MyLambdaRole</Arn>
      <CreateDate>2026-03-19T12:00:00Z</CreateDate>
      <AssumeRolePolicyDocument>%7B%22Version%22%3A%222012-10-17%22...%7D</AssumeRolePolicyDocument>
    </Role>
  </CreateRoleResult>
  <ResponseMetadata>
    <RequestId>7a62c49f-347e-4fc4-9331-6e8eEXAMPLE</RequestId>
  </ResponseMetadata>
</CreateRoleResponse>
```

**AttachRolePolicy request:**

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded

Action=AttachRolePolicy
&RoleName=MyLambdaRole
&PolicyArn=arn%3Aaws%3Aiam%3A%3Aaws%3Apolicy%2Fservice-role%2FAWSLambdaBasicExecutionRole
&Version=2010-05-08
```

**AttachRolePolicy response:**

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<AttachRolePolicyResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
  <ResponseMetadata>
    <RequestId>7a62c49f-347e-4fc4-9331-6e8eEXAMPLE</RequestId>
  </ResponseMetadata>
</AttachRolePolicyResponse>
```

**Error response:**

```http
HTTP/1.1 404 Not Found
Content-Type: text/xml

<ErrorResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
  <Error>
    <Type>Sender</Type>
    <Code>NoSuchEntity</Code>
    <Message>The role with name MyRole cannot be found.</Message>
  </Error>
  <RequestId>7a62c49f-347e-4fc4-9331-6e8eEXAMPLE</RequestId>
</ErrorResponse>
```

### 5.4 Form Parameter Encoding Conventions

IAM uses the same form encoding conventions as SNS:

**Flat parameters:**
```
Action=CreateUser&UserName=testuser&Path=/developers/&Version=2010-05-08
```

**List parameters (Tags):**
```
Tags.member.1.Key=Department&Tags.member.1.Value=Engineering
&Tags.member.2.Key=Project&Tags.member.2.Value=Rustack
```

**Policy document (URL-encoded JSON string):**
```
PolicyDocument=%7B%22Version%22%3A%222012-10-17%22%2C%22Statement%22%3A...%7D
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-iam-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/iam.json` (IAM namespace `com.amazonaws.iam`, ~60+ operations)
**Service config:** `codegen/services/iam.toml`
**Generate:** `make codegen-iam`

### 6.2 Proposed `codegen/services/iam.toml`

```toml
[service]
name = "iam"
display_name = "IAM"
rust_prefix = "Iam"
namespace = "com.amazonaws.iam"
protocol = "awsQuery"

[protocol]
serde_rename = "PascalCase"
emit_serde_derives = true

[operations]
phase0 = [
    # User management
    "CreateUser", "GetUser", "DeleteUser", "ListUsers", "UpdateUser",
    # Role management
    "CreateRole", "GetRole", "DeleteRole", "ListRoles", "UpdateRole",
    # Managed policy management
    "CreatePolicy", "GetPolicy", "DeletePolicy", "ListPolicies",
    # Policy attachment - users
    "AttachUserPolicy", "DetachUserPolicy", "ListAttachedUserPolicies",
    # Policy attachment - roles
    "AttachRolePolicy", "DetachRolePolicy", "ListAttachedRolePolicies",
    # Access key management
    "CreateAccessKey", "DeleteAccessKey", "ListAccessKeys", "UpdateAccessKey",
    "GetAccessKeyLastUsed",
]
phase1 = [
    # Group management
    "CreateGroup", "GetGroup", "DeleteGroup", "ListGroups", "UpdateGroup",
    # Group membership
    "AddUserToGroup", "RemoveUserFromGroup", "ListGroupsForUser",
    # Policy attachment - groups
    "AttachGroupPolicy", "DetachGroupPolicy", "ListAttachedGroupPolicies",
    # Instance profile management
    "CreateInstanceProfile", "GetInstanceProfile", "DeleteInstanceProfile",
    "ListInstanceProfiles", "ListInstanceProfilesForRole",
    "AddRoleToInstanceProfile", "RemoveRoleFromInstanceProfile",
]
phase2 = [
    # Policy versions
    "CreatePolicyVersion", "GetPolicyVersion", "DeletePolicyVersion",
    "ListPolicyVersions", "SetDefaultPolicyVersion",
    # Inline policies - users
    "PutUserPolicy", "GetUserPolicy", "DeleteUserPolicy", "ListUserPolicies",
    # Inline policies - roles
    "PutRolePolicy", "GetRolePolicy", "DeleteRolePolicy", "ListRolePolicies",
    # Inline policies - groups
    "PutGroupPolicy", "GetGroupPolicy", "DeleteGroupPolicy", "ListGroupPolicies",
]
phase3 = [
    # Tagging - users
    "TagUser", "UntagUser", "ListUserTags",
    # Tagging - roles
    "TagRole", "UntagRole", "ListRoleTags",
    # Service-linked roles
    "CreateServiceLinkedRole", "DeleteServiceLinkedRole",
    "GetServiceLinkedRoleDeletionStatus",
    # Trust policy
    "UpdateAssumeRolePolicy",
    # Policy simulation (stubs)
    "SimulatePrincipalPolicy", "SimulateCustomPolicy",
    # Entity details
    "ListEntitiesForPolicy",
    "GetAccountAuthorizationDetails",
]

[errors.custom]
NoSuchEntity = { status = 404, message = "The specified entity cannot be found" }
EntityAlreadyExists = { status = 409, message = "The entity already exists" }
LimitExceeded = { status = 409, message = "The request was rejected because it attempted to create resources beyond the current account limits" }
DeleteConflict = { status = 409, message = "The request was rejected because it attempted to delete a resource that has attached subordinate entities" }
MalformedPolicyDocument = { status = 400, message = "The policy document is malformed" }
InvalidInput = { status = 400, message = "The request was rejected because an invalid or out-of-range value was supplied for an input parameter" }
MissingAction = { status = 400, message = "Missing action parameter" }
InvalidAction = { status = 400, message = "The action is not valid for this endpoint" }
ServiceFailure = { status = 500, message = "The request processing has failed because of an unknown error, exception or failure" }

[output]
file_layout = "flat"
```

### 6.3 Generated Output

The codegen produces 6 files in `crates/rustack-iam-model/src/`:

| File            | Contents                                                                                                                             |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `lib.rs`        | Module declarations and re-exports                                                                                                   |
| `types.rs`      | Shared types (User, Role, Group, Policy, InstanceProfile, AccessKey, AccessKeyMetadata, Tag, PolicyVersion, etc.) with serde derives |
| `operations.rs` | `IamOperation` enum with `as_str()`, `from_name()`, phase methods                                                                    |
| `error.rs`      | `IamErrorCode` enum + `IamError` struct + `iam_error!` macro                                                                         |
| `input.rs`      | All input structs with `#[serde(rename_all = "PascalCase")]`                                                                         |
| `output.rs`     | All output structs with serde derives                                                                                                |

### 6.4 Service-Specific Notes

IAM has several codegen considerations:

1. **Timestamp format**: IAM uses ISO 8601 timestamps (`2026-03-19T12:00:00Z`) in XML responses, unlike Secrets Manager (epoch seconds) or SNS (no timestamps). The model types should use `String` for timestamp fields, with formatting handled in the core layer.
2. **URL-encoded policy documents in responses**: `AssumeRolePolicyDocument` is returned URL-encoded in XML responses. This is an IAM-specific behavior.
3. **Path prefix**: Many IAM entities support a hierarchical `Path` (e.g., `/developers/team-a/`). Paths default to `/` and are included in ARNs.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-iam-model` (auto-generated)

```
crates/rustack-iam-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: User, Role, Group, Policy, InstanceProfile,
    |                       #   AccessKey, AccessKeyMetadata, PolicyVersion, Tag, etc.
    +-- operations.rs       # Auto-generated: IamOperation enum (~60 variants)
    +-- error.rs            # Auto-generated: IamErrorCode enum + IamError struct
    +-- input.rs            # Auto-generated: all ~60 input structs
    +-- output.rs           # Auto-generated: all ~60 output structs
```

**Dependencies:** `serde`, `serde_json`

### 7.2 `rustack-iam-core`

```
crates/rustack-iam-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # IamConfig
    +-- handler.rs          # IamHandler trait (all operation dispatch)
    +-- provider.rs         # RustackIam (main provider, all operation handlers)
    +-- storage.rs          # IamStore: top-level store with all DashMaps
    +-- types.rs            # Internal data types: UserRecord, RoleRecord, GroupRecord,
    |                       #   ManagedPolicyRecord, InstanceProfileRecord, AccessKeyRecord
    +-- arn.rs              # ARN generation for all IAM entity types
    +-- id_gen.rs           # Unique ID generation (AIDA/AROA/AGPA/ANPA/AIPA prefixed IDs)
    +-- validation.rs       # Entity name, path, policy document validation
    +-- users.rs            # User CRUD operations
    +-- roles.rs            # Role CRUD operations
    +-- groups.rs           # Group CRUD + membership operations
    +-- policies.rs         # Managed policy CRUD + versioning operations
    +-- inline_policies.rs  # Inline policy CRUD (put/get/delete/list for user/role/group)
    +-- attachments.rs      # Policy attachment/detachment operations
    +-- instance_profiles.rs # Instance profile CRUD operations
    +-- access_keys.rs      # Access key CRUD operations
    +-- tags.rs             # Tag/Untag/ListTags operations
    +-- service_linked.rs   # Service-linked role operations
```

**Dependencies:** `rustack-core`, `rustack-iam-model`, `dashmap`, `serde_json`, `tracing`, `rand`, `chrono`

### 7.3 `rustack-iam-http`

```
crates/rustack-iam-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # Action= parameter dispatch to IamOperation
    +-- service.rs          # IamHttpService (hyper Service impl)
    +-- dispatch.rs         # IamHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- request.rs          # Form parameter parsing
    +-- response.rs         # XML response construction, XmlWriter, error formatting
```

**Dependencies:** `rustack-iam-model`, `rustack-auth`, `hyper`, `http`, `bytes`, `serde_urlencoded`, `uuid`

This crate is structurally identical to `rustack-sns-http`. The router parses `Action=<IamOp>` from the form body and dispatches to the handler. XML responses use a different namespace (`https://iam.amazonaws.com/doc/2010-05-08/`).

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-iam-model = { path = "crates/rustack-iam-model" }
rustack-iam-http = { path = "crates/rustack-iam-http" }
rustack-iam-core = { path = "crates/rustack-iam-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// IAM operation router.
///
/// Parses the `Action=<Op>` parameter from the form body to determine the
/// IAM operation.
pub fn resolve_operation(params: &[(String, String)]) -> Result<IamOperation, IamError> {
    let action = params
        .iter()
        .find(|(k, _)| k == "Action")
        .map(|(_, v)| v.as_str())
        .ok_or_else(IamError::missing_action)?;

    IamOperation::from_name(action).ok_or_else(|| IamError::unknown_operation(action))
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// Routes requests to the IAM service.
///
/// Matches `POST /` requests with `Content-Type: application/x-www-form-urlencoded`
/// where the SigV4 credential scope service is `iam`, or where the `Action=`
/// parameter is a recognized IAM operation.
pub struct IamServiceRouter<H: IamHandler> {
    inner: IamHttpService<H>,
}

impl<H: IamHandler> ServiceRouter for IamServiceRouter<H> {
    fn name(&self) -> &'static str {
        "iam"
    }

    /// IAM matches form-urlencoded POST requests signed with service=iam.
    ///
    /// This MUST be registered before the SNS router in the service list,
    /// because both IAM and SNS match on Content-Type. The routing order is:
    /// 1. Check SigV4 service name in Authorization header
    /// 2. If service=iam, match
    /// 3. If no Authorization header, fall through to SNS (which does Action-based matching)
    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        if *req.method() != http::Method::POST {
            return false;
        }

        let is_form = req.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("x-www-form-urlencoded"));

        if !is_form {
            return false;
        }

        // Primary: check SigV4 service name
        if let Some(service) = extract_sigv4_service(req) {
            return service == "iam";
        }

        // Cannot determine from headers alone; will need body inspection.
        // Return false here and let the gateway handle body-based routing.
        false
    }

    fn call(
        &self,
        req: http::Request<Incoming>,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
    {
        let svc = self.inner.clone();
        Box::pin(async move {
            let resp = svc.call(req).await;
            Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
        })
    }
}

/// Extract the SigV4 service name from the Authorization header.
fn extract_sigv4_service(req: &http::Request<Incoming>) -> Option<&str> {
    let auth = req.headers().get("authorization")?.to_str().ok()?;
    let cred_start = auth.find("Credential=")? + "Credential=".len();
    let cred_end = auth[cred_start..].find(',')?;
    let credential = &auth[cred_start..cred_start + cred_end];
    let parts: Vec<&str> = credential.split('/').collect();
    // parts = ["AKID", "date", "region", "SERVICE", "aws4_request"]
    parts.get(3).copied()
}
```

### 8.3 Handler Trait

```rust
/// Trait that the IAM business logic provider must implement.
///
/// The handler receives a parsed operation enum and the raw form body bytes.
/// This follows the same pattern as the SNS handler.
pub trait IamHandler: Send + Sync + 'static {
    /// Handle an IAM operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: IamOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<IamResponseBody>, IamError>> + Send>>;
}
```

### 8.4 XML Response Builder

IAM XML responses follow the same pattern as SNS but with a different namespace:

```rust
/// The IAM XML namespace.
const XML_NS: &str = "https://iam.amazonaws.com/doc/2010-05-08/";

/// XML writer for building IAM response XML.
///
/// Reuses the same pattern as SNS's XmlWriter but with the IAM namespace.
pub struct IamXmlWriter {
    buf: String,
}

impl IamXmlWriter {
    /// Start the response envelope: `<{op}Response xmlns="...">`.
    pub fn start_response(&mut self, operation: &str) {
        self.buf.push('<');
        self.buf.push_str(operation);
        self.buf.push_str("Response xmlns=\"");
        self.buf.push_str(XML_NS);
        self.buf.push_str("\">");
    }

    /// Start the result element: `<{op}Result>`.
    pub fn start_result(&mut self, operation: &str) {
        self.buf.push('<');
        self.buf.push_str(operation);
        self.buf.push_str("Result>");
    }

    // ... same API as SNS XmlWriter: end_element, write_element,
    //     write_optional_element, write_bool_element, write_response_metadata

    /// Write a member list: `<member>...</member>` for each item.
    pub fn write_members<F>(&mut self, tag: &str, items: &[impl std::fmt::Debug], write_fn: F)
    where
        F: Fn(&mut Self, &dyn std::fmt::Debug),
    {
        self.buf.push('<');
        self.buf.push_str(tag);
        self.buf.push('>');
        for item in items {
            self.buf.push_str("<member>");
            write_fn(self, item);
            self.buf.push_str("</member>");
        }
        self.buf.push_str("</");
        self.buf.push_str(tag);
        self.buf.push('>');
    }
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

IAM is a **global** service. Unlike regional services (S3, DynamoDB, SQS, etc.) that partition state by `(account_id, region)`, IAM stores all entities under `account_id` only. In Rustack's single-account model, this means a single flat namespace for all IAM entities.

The storage model consists of 6 primary entity stores (DashMaps) plus relationship tracking. The entity model has these key relationships:

```
User ──┬── belongs to ──> Group (many-to-many)
       ├── has ──> AccessKey (one-to-many)
       ├── has attached ──> ManagedPolicy (many-to-many)
       └── has inline ──> InlinePolicy (one-to-many)

Role ──┬── has attached ──> ManagedPolicy (many-to-many)
       ├── has inline ──> InlinePolicy (one-to-many)
       └── belongs to ──> InstanceProfile (many-to-many, but typically 1:1)

Group ──┬── has members ──> User (many-to-many)
        ├── has attached ──> ManagedPolicy (many-to-many)
        └── has inline ──> InlinePolicy (one-to-many)

ManagedPolicy ──> has versions ──> PolicyVersion (one-to-many, max 5)

InstanceProfile ──> contains ──> Role (one-to-many, but typically 0 or 1)
```

### 9.2 Core Data Structures

```rust
use std::collections::{HashMap, HashSet};

use dashmap::DashMap;

/// Top-level IAM store.
///
/// IAM is a global service -- entities are keyed by account_id only,
/// not by (account_id, region). In Rustack's single-account model,
/// this is a flat namespace.
pub struct IamStore {
    /// Users keyed by username.
    pub users: DashMap<String, UserRecord>,
    /// Roles keyed by role name.
    pub roles: DashMap<String, RoleRecord>,
    /// Groups keyed by group name.
    pub groups: DashMap<String, GroupRecord>,
    /// Managed policies keyed by policy ARN.
    pub policies: DashMap<String, ManagedPolicyRecord>,
    /// Instance profiles keyed by instance profile name.
    pub instance_profiles: DashMap<String, InstanceProfileRecord>,
    /// Access keys keyed by access key ID.
    pub access_keys: DashMap<String, AccessKeyRecord>,
}

/// An IAM user.
pub struct UserRecord {
    /// Username (1-64 chars, alphanumeric plus `+=,.@_-`).
    pub user_name: String,
    /// Unique user ID (e.g., `AIDAEXAMPLEID`).
    pub user_id: String,
    /// User ARN (e.g., `arn:aws:iam::000000000000:user/path/username`).
    pub arn: String,
    /// Path prefix (default `/`).
    pub path: String,
    /// When the user was created (ISO 8601).
    pub create_date: String,
    /// Tags on the user.
    pub tags: Vec<Tag>,
    /// Permissions boundary policy ARN (optional).
    pub permissions_boundary: Option<String>,

    // -- Relationships --
    /// Managed policy ARNs attached to this user.
    pub attached_policies: HashSet<String>,
    /// Inline policies keyed by policy name.
    pub inline_policies: HashMap<String, String>,
    /// Group names this user belongs to.
    pub groups: HashSet<String>,
}

/// An IAM role.
pub struct RoleRecord {
    /// Role name (1-64 chars).
    pub role_name: String,
    /// Unique role ID (e.g., `AROAEXAMPLEID`).
    pub role_id: String,
    /// Role ARN.
    pub arn: String,
    /// Path prefix (default `/`).
    pub path: String,
    /// Trust policy document (JSON string).
    pub assume_role_policy_document: String,
    /// Description (optional).
    pub description: Option<String>,
    /// Maximum session duration in seconds (default 3600).
    pub max_session_duration: i32,
    /// When the role was created (ISO 8601).
    pub create_date: String,
    /// Tags on the role.
    pub tags: Vec<Tag>,
    /// Permissions boundary policy ARN (optional).
    pub permissions_boundary: Option<String>,

    // -- Relationships --
    /// Managed policy ARNs attached to this role.
    pub attached_policies: HashSet<String>,
    /// Inline policies keyed by policy name.
    pub inline_policies: HashMap<String, String>,
    /// Whether this is a service-linked role.
    pub is_service_linked: bool,
}

/// An IAM group.
pub struct GroupRecord {
    /// Group name.
    pub group_name: String,
    /// Unique group ID (e.g., `AGPAEXAMPLEID`).
    pub group_id: String,
    /// Group ARN.
    pub arn: String,
    /// Path prefix.
    pub path: String,
    /// When the group was created.
    pub create_date: String,

    // -- Relationships --
    /// Managed policy ARNs attached to this group.
    pub attached_policies: HashSet<String>,
    /// Inline policies keyed by policy name.
    pub inline_policies: HashMap<String, String>,
    /// Usernames that are members of this group.
    pub members: HashSet<String>,
}

/// A managed IAM policy.
pub struct ManagedPolicyRecord {
    /// Policy name.
    pub policy_name: String,
    /// Unique policy ID (e.g., `ANPAEXAMPLEID`).
    pub policy_id: String,
    /// Policy ARN.
    pub arn: String,
    /// Path prefix.
    pub path: String,
    /// Description (optional).
    pub description: Option<String>,
    /// When the policy was created.
    pub create_date: String,
    /// When the policy was last updated.
    pub update_date: String,
    /// Whether this is an AWS-managed policy.
    pub is_attachable: bool,
    /// Number of entities this policy is attached to.
    pub attachment_count: i32,
    /// Number of permissions boundaries using this policy.
    pub permissions_boundary_usage_count: i32,

    // -- Versioning --
    /// Policy versions keyed by version ID (e.g., "v1", "v2").
    pub versions: Vec<PolicyVersionRecord>,
    /// The default version ID.
    pub default_version_id: String,

    // -- Tags --
    pub tags: Vec<Tag>,
}

/// A single version of a managed policy.
pub struct PolicyVersionRecord {
    /// Version ID (e.g., "v1", "v2", up to "v5").
    pub version_id: String,
    /// The policy document JSON string.
    pub document: String,
    /// Whether this is the default version.
    pub is_default_version: bool,
    /// When this version was created.
    pub create_date: String,
}

/// An IAM instance profile.
pub struct InstanceProfileRecord {
    /// Instance profile name.
    pub instance_profile_name: String,
    /// Unique instance profile ID (e.g., `AIPAEXAMPLEID`).
    pub instance_profile_id: String,
    /// Instance profile ARN.
    pub arn: String,
    /// Path prefix.
    pub path: String,
    /// When the instance profile was created.
    pub create_date: String,
    /// Tags.
    pub tags: Vec<Tag>,

    // -- Relationships --
    /// Role names contained in this instance profile.
    /// AWS allows at most 1 role per instance profile.
    pub roles: Vec<String>,
}

/// An IAM access key.
pub struct AccessKeyRecord {
    /// The access key ID (e.g., `AKIAEXAMPLEID`).
    pub access_key_id: String,
    /// The secret access key (generated, returned only on creation).
    pub secret_access_key: String,
    /// The username this key belongs to.
    pub user_name: String,
    /// Status: Active or Inactive.
    pub status: AccessKeyStatus,
    /// When the key was created.
    pub create_date: String,
    /// When the key was last used (optional).
    pub last_used_date: Option<String>,
    /// Service last used with (optional).
    pub last_used_service: Option<String>,
    /// Region last used in (optional).
    pub last_used_region: Option<String>,
}

/// Access key status.
pub enum AccessKeyStatus {
    Active,
    Inactive,
}

/// Tag structure (reused across all entity types).
pub struct Tag {
    pub key: String,
    pub value: String,
}
```

### 9.3 IAM Entity ID Generation

AWS IAM generates unique IDs with specific prefixes for each entity type. These IDs are 21 characters long: a 4-character prefix + 17 alphanumeric characters.

| Entity Type      | Prefix | Example                 |
| ---------------- | ------ | ----------------------- |
| User             | `AIDA` | `AIDAEXAMPLE1234567890` |
| Role             | `AROA` | `AROAEXAMPLE1234567890` |
| Group            | `AGPA` | `AGPAEXAMPLE1234567890` |
| Managed Policy   | `ANPA` | `ANPAEXAMPLE1234567890` |
| Instance Profile | `AIPA` | `AIPAEXAMPLE1234567890` |
| Access Key ID    | `AKIA` | `AKIAEXAMPLE12345`      |

```rust
/// Generate a unique IAM entity ID with the given prefix.
///
/// IAM IDs are 21 characters: 4-char prefix + 17 uppercase alphanumeric chars.
fn generate_iam_id(prefix: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let suffix: String = (0..17)
        .map(|_| {
            let idx = rand::random::<usize>() % CHARS.len();
            CHARS[idx] as char
        })
        .collect();
    format!("{prefix}{suffix}")
}

/// Generate an access key ID (starts with AKIA, 20 chars total).
fn generate_access_key_id() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let suffix: String = (0..16)
        .map(|_| {
            let idx = rand::random::<usize>() % CHARS.len();
            CHARS[idx] as char
        })
        .collect();
    format!("AKIA{suffix}")
}

/// Generate a secret access key (40 chars, mixed case alphanumeric + /+).
fn generate_secret_access_key() -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789/+";
    (0..40)
        .map(|_| {
            let idx = rand::random::<usize>() % CHARS.len();
            CHARS[idx] as char
        })
        .collect()
}
```

### 9.4 ARN Construction

IAM ARNs have specific formats for each entity type. Note that IAM ARNs have no region component (IAM is global):

```rust
/// Generate an IAM ARN.
///
/// IAM ARNs have no region: `arn:aws:iam::{account_id}:{resource_type}/{path}{name}`
fn iam_arn(account_id: &str, resource_type: &str, path: &str, name: &str) -> String {
    // Paths always start and end with /
    // For default path (/), the ARN is: arn:aws:iam::123456789012:user/myuser
    // For custom path (/dev/), the ARN is: arn:aws:iam::123456789012:user/dev/myuser
    if path == "/" {
        format!("arn:aws:iam::{account_id}:{resource_type}/{name}")
    } else {
        // path already starts with / and ends with /
        let path_trimmed = path.trim_start_matches('/');
        format!("arn:aws:iam::{account_id}:{resource_type}/{path_trimmed}{name}")
    }
}

// Examples:
// iam_arn("000000000000", "user", "/", "testuser")
//   -> "arn:aws:iam::000000000000:user/testuser"
// iam_arn("000000000000", "role", "/service-role/", "MyLambdaRole")
//   -> "arn:aws:iam::000000000000:role/service-role/MyLambdaRole"
// iam_arn("000000000000", "policy", "/", "MyPolicy")
//   -> "arn:aws:iam::000000000000:policy/MyPolicy"
// iam_arn("000000000000", "instance-profile", "/", "MyProfile")
//   -> "arn:aws:iam::000000000000:instance-profile/MyProfile"
```

### 9.5 Managed Policies vs Inline Policies

IAM has two types of policies:

**Managed Policies:**
- Standalone entities with their own ARN, stored in `IamStore.policies`
- Can be attached to multiple users, roles, and groups
- Support versioning (up to 5 versions, one default)
- Two sub-types: **AWS-managed** (`arn:aws:iam::aws:policy/...`) and **customer-managed** (`arn:aws:iam::{account_id}:policy/...`)
- Created via `CreatePolicy`, attached via `Attach{User,Role,Group}Policy`

**Inline Policies:**
- Embedded directly in a user, role, or group record
- Not shared across entities; each entity has its own copy
- No versioning
- Created via `Put{User,Role,Group}Policy`
- Stored in `UserRecord.inline_policies`, `RoleRecord.inline_policies`, `GroupRecord.inline_policies` as `HashMap<String, String>` (policy_name -> document)

For Rustack's local dev use case, we store policy documents as JSON strings without parsing or evaluating them. The policy document is validated for JSON syntax only.

### 9.6 Policy Attachment Tracking

Managed policy attachments create many-to-many relationships. We track these bidirectionally:

- **Entity side**: `UserRecord.attached_policies`, `RoleRecord.attached_policies`, `GroupRecord.attached_policies` contain the policy ARN
- **Policy side**: `ManagedPolicyRecord.attachment_count` tracks the total count (needed for `GetPolicy` response and `DeletePolicy` validation)

When detaching or deleting an entity, we must update both sides:

```rust
impl IamStore {
    /// Attach a managed policy to a role.
    pub fn attach_role_policy(
        &self,
        role_name: &str,
        policy_arn: &str,
    ) -> Result<(), IamError> {
        // Validate role exists
        let mut role = self.roles.get_mut(role_name)
            .ok_or_else(|| IamError::no_such_entity(
                format!("The role with name {role_name} cannot be found."),
            ))?;

        // Validate policy exists
        let mut policy = self.policies.get_mut(policy_arn)
            .ok_or_else(|| IamError::no_such_entity(
                format!("Policy {policy_arn} does not exist."),
            ))?;

        // Check attachment limit (20 per entity by default)
        if role.attached_policies.len() >= 20 {
            return Err(IamError::limit_exceeded(
                "Cannot exceed quota for PoliciesPerRole: 20",
            ));
        }

        // Attach (idempotent: no error if already attached)
        if role.attached_policies.insert(policy_arn.to_string()) {
            policy.attachment_count += 1;
        }

        Ok(())
    }

    /// Delete a managed policy. Fails if any entities are still attached.
    pub fn delete_policy(&self, policy_arn: &str) -> Result<(), IamError> {
        let policy = self.policies.get(policy_arn)
            .ok_or_else(|| IamError::no_such_entity(
                format!("Policy {policy_arn} was not found."),
            ))?;

        if policy.attachment_count > 0 {
            return Err(IamError::delete_conflict(
                "Cannot delete a policy attached to entities. Detach the policy first.",
            ));
        }

        drop(policy); // release read lock
        self.policies.remove(policy_arn);
        Ok(())
    }
}
```

### 9.7 Concurrency Model

IAM is a pure request/response service with no streaming, no background processing, and no cross-service integration. `DashMap` provides sufficient concurrent access:

- **Reads** (GetUser, GetRole, ListPolicies, etc.): lock-free concurrent reads
- **Writes** (CreateUser, AttachRolePolicy, etc.): per-entry write locks via DashMap
- **Cross-entity writes** (AttachRolePolicy updates both role and policy): acquire DashMap entries sequentially. Since IAM operations in local dev are not high-contention, simple sequential locking is sufficient. Always acquire locks in a consistent order (alphabetical by entity type: policy first, then role) to avoid deadlocks.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main IAM provider implementing all operations.
pub struct RustackIam {
    pub(crate) store: Arc<IamStore>,
    pub(crate) config: Arc<IamConfig>,
}

impl RustackIam {
    pub fn new(config: IamConfig) -> Self {
        Self {
            store: Arc::new(IamStore::new()),
            config: Arc::new(config),
        }
    }
}
```

### 10.2 Operations

#### Phase 0: Core CRUD (~25 operations)

**CreateUser** -- Create a new IAM user.

1. Validate `UserName` (1-64 chars, alphanumeric plus `+=,.@_-`)
2. Validate `Path` if provided (1-512 chars, must start and end with `/`)
3. Check if user with same name exists; if so, return `EntityAlreadyExists`
4. Generate unique user ID (`AIDA` prefix)
5. Generate user ARN
6. If `Tags` provided, validate and store (max 50 tags)
7. If `PermissionsBoundary` provided, validate policy ARN exists and store
8. Store `UserRecord`
9. Return `{ User: { UserName, UserId, Arn, Path, CreateDate, Tags, PermissionsBoundary } }`

**GetUser** -- Get details about a user.

1. If `UserName` provided, look up that user
2. If `UserName` not provided, return the "current" user (use default user for local dev)
3. If user not found, return `NoSuchEntity`
4. Return `{ User: { UserName, UserId, Arn, Path, CreateDate, Tags, PermissionsBoundary } }`

**DeleteUser** -- Delete a user.

1. Look up user by name; if not found, return `NoSuchEntity`
2. Validate user has no attached policies (if any, return `DeleteConflict`)
3. Validate user has no inline policies (if any, return `DeleteConflict`)
4. Validate user has no group memberships (if any, return `DeleteConflict`)
5. Validate user has no access keys (if any, return `DeleteConflict`)
6. Remove user from store
7. Return `{}`

**ListUsers** -- List all users.

1. Iterate all users in store
2. If `PathPrefix` provided, filter by path prefix
3. Sort by username (case-sensitive)
4. Paginate with `MaxItems` (1-1000, default 100) and `Marker`
5. Return `{ Users: [...], IsTruncated, Marker }`

**UpdateUser** -- Update a user's name or path.

1. Look up user by `UserName`; if not found, return `NoSuchEntity`
2. If `NewUserName` provided, validate and rename (update user record, update all group memberships, update access key records)
3. If `NewPath` provided, update path and regenerate ARN
4. Return `{}`

**CreateRole** -- Create a new IAM role.

1. Validate `RoleName` (1-64 chars)
2. Validate `AssumeRolePolicyDocument` is valid JSON (do not parse policy language)
3. Validate `Path` if provided
4. Check if role with same name exists; if so, return `EntityAlreadyExists`
5. Generate unique role ID (`AROA` prefix) and ARN
6. Set `MaxSessionDuration` (default 3600, range 3600-43200)
7. If `Tags` provided, store tags
8. If `PermissionsBoundary` provided, validate and store
9. Store `RoleRecord`
10. Return `{ Role: { RoleName, RoleId, Arn, Path, AssumeRolePolicyDocument, CreateDate, ... } }`

**GetRole** -- Get details about a role.

1. Look up role by name; if not found, return `NoSuchEntity`
2. Return `{ Role: { RoleName, RoleId, Arn, Path, AssumeRolePolicyDocument, Description, MaxSessionDuration, CreateDate, Tags, PermissionsBoundary } }`

**DeleteRole** -- Delete a role.

1. Look up role by name; if not found, return `NoSuchEntity`
2. Validate role has no attached policies (return `DeleteConflict`)
3. Validate role has no inline policies (return `DeleteConflict`)
4. Validate role is not in any instance profiles (return `DeleteConflict`)
5. Remove role from store
6. Return `{}`

**ListRoles** -- List all roles.

1. Iterate all roles in store
2. If `PathPrefix` provided, filter by path prefix
3. Sort by role name
4. Paginate with `MaxItems` and `Marker`
5. Return `{ Roles: [...], IsTruncated, Marker }`

**UpdateRole** -- Update a role's description or max session duration.

1. Look up role by name; if not found, return `NoSuchEntity`
2. If `Description` provided, update
3. If `MaxSessionDuration` provided, validate range and update
4. Return `{}`

**CreatePolicy** -- Create a managed policy.

1. Validate `PolicyName` and `PolicyDocument` (valid JSON)
2. Validate `Path` if provided
3. Generate policy ARN: `arn:aws:iam::{account_id}:policy/{path}{name}`
4. Check if policy with same ARN exists; if so, return `EntityAlreadyExists`
5. Generate unique policy ID (`ANPA` prefix)
6. Create initial policy version (v1, marked as default)
7. If `Tags` provided, store tags
8. Store `ManagedPolicyRecord`
9. Return `{ Policy: { PolicyName, PolicyId, Arn, Path, DefaultVersionId, AttachmentCount, CreateDate, UpdateDate, ... } }`

**GetPolicy** -- Get metadata about a managed policy.

1. Look up policy by ARN; if not found, return `NoSuchEntity`
2. Return `{ Policy: { PolicyName, PolicyId, Arn, DefaultVersionId, AttachmentCount, CreateDate, UpdateDate, Description, ... } }`

**DeletePolicy** -- Delete a managed policy.

1. Look up policy by ARN; if not found, return `NoSuchEntity`
2. Validate `attachment_count == 0` (return `DeleteConflict`)
3. Remove policy from store
4. Return `{}`

**ListPolicies** -- List managed policies.

1. Iterate all policies in store
2. Apply filters: `Scope` (All/AWS/Local), `PathPrefix`, `PolicyUsageFilter` (PermissionsPolicy/PermissionsBoundary), `OnlyAttached`
3. Sort by policy name
4. Paginate with `MaxItems` and `Marker`
5. Return `{ Policies: [...], IsTruncated, Marker }`

**AttachUserPolicy** -- Attach a managed policy to a user.

1. Validate user exists
2. Validate policy exists
3. Check attachment limit (20 policies per user)
4. Add policy ARN to `UserRecord.attached_policies`
5. Increment `ManagedPolicyRecord.attachment_count`
6. Return `{}`

**DetachUserPolicy** -- Detach a managed policy from a user.

1. Validate user exists
2. Validate policy exists
3. Validate policy is currently attached to user (return `NoSuchEntity` if not)
4. Remove policy ARN from `UserRecord.attached_policies`
5. Decrement `ManagedPolicyRecord.attachment_count`
6. Return `{}`

**ListAttachedUserPolicies** -- List managed policies attached to a user.

1. Validate user exists
2. Return `{ AttachedPolicies: [{ PolicyName, PolicyArn }], IsTruncated, Marker }`

**AttachRolePolicy** -- Attach a managed policy to a role.

1. Same pattern as AttachUserPolicy but for roles
2. Check attachment limit (20 policies per role)
3. Return `{}`

**DetachRolePolicy** -- Detach a managed policy from a role.

1. Same pattern as DetachUserPolicy but for roles
2. Return `{}`

**ListAttachedRolePolicies** -- List managed policies attached to a role.

1. Validate role exists
2. If `PathPrefix` provided, filter policies by path
3. Return `{ AttachedPolicies: [{ PolicyName, PolicyArn }], IsTruncated, Marker }`

**CreateAccessKey** -- Create a new access key for a user.

1. Validate user exists
2. Check access key limit (2 per user)
3. Generate access key ID (`AKIA` prefix, 20 chars)
4. Generate secret access key (40 chars)
5. Store `AccessKeyRecord` keyed by access key ID
6. Return `{ AccessKey: { UserName, AccessKeyId, Status: Active, SecretAccessKey, CreateDate } }`
   - Note: `SecretAccessKey` is returned ONLY in the CreateAccessKey response. It cannot be retrieved later.

**DeleteAccessKey** -- Delete an access key.

1. Look up access key by ID; if not found, return `NoSuchEntity`
2. Validate the key belongs to the specified user
3. Remove from store
4. Return `{}`

**ListAccessKeys** -- List access keys for a user.

1. Validate user exists
2. Iterate all access keys, filter by username
3. Return `{ AccessKeyMetadata: [{ UserName, AccessKeyId, Status, CreateDate }], IsTruncated, Marker }`

**UpdateAccessKey** -- Enable or disable an access key.

1. Look up access key by ID; if not found, return `NoSuchEntity`
2. Validate the key belongs to the specified user
3. Update status to Active or Inactive
4. Return `{}`

**GetAccessKeyLastUsed** -- Get last usage info for an access key.

1. Look up access key by ID; if not found, return `NoSuchEntity`
2. Return `{ UserName, AccessKeyLastUsed: { LastUsedDate, ServiceName, Region } }`
   - In local dev, return "N/A" for ServiceName and Region if never used

#### Phase 1: Groups + Instance Profiles (~17 operations)

**CreateGroup** -- Create a new IAM group.

1. Validate `GroupName` (1-128 chars)
2. Check if group exists; return `EntityAlreadyExists` if so
3. Generate unique group ID (`AGPA` prefix) and ARN
4. Store `GroupRecord`
5. Return `{ Group: { GroupName, GroupId, Arn, Path, CreateDate } }`

**GetGroup** -- Get group details and member list.

1. Look up group; return `NoSuchEntity` if not found
2. Return `{ Group: { GroupName, GroupId, Arn, Path, CreateDate }, Users: [{ UserName, UserId, Arn, ... }], IsTruncated, Marker }`

**DeleteGroup** -- Delete a group.

1. Validate group has no members (`DeleteConflict`)
2. Validate group has no attached policies (`DeleteConflict`)
3. Validate group has no inline policies (`DeleteConflict`)
4. Remove group from store
5. Return `{}`

**ListGroups** -- List all groups.

1. Paginate with path prefix filtering
2. Return `{ Groups: [...], IsTruncated, Marker }`

**UpdateGroup** -- Update group name or path.

1. Look up group; update name/path, regenerate ARN
2. If renaming, update all user records that reference this group
3. Return `{}`

**AddUserToGroup** -- Add a user to a group.

1. Validate both user and group exist
2. Add username to `GroupRecord.members`
3. Add group name to `UserRecord.groups`
4. Return `{}` (idempotent: no error if already a member)

**RemoveUserFromGroup** -- Remove a user from a group.

1. Validate both user and group exist
2. Validate user is a member of the group
3. Remove from both sides
4. Return `{}`

**ListGroupsForUser** -- List groups a user belongs to.

1. Validate user exists
2. Return `{ Groups: [{ GroupName, GroupId, Arn, Path, CreateDate }] }`

**AttachGroupPolicy / DetachGroupPolicy / ListAttachedGroupPolicies** -- Same patterns as user/role policy attachment but for groups.

**CreateInstanceProfile** -- Create an instance profile.

1. Validate name; check uniqueness
2. Generate unique ID (`AIPA` prefix) and ARN
3. Store `InstanceProfileRecord` with empty roles list
4. Return `{ InstanceProfile: { InstanceProfileName, InstanceProfileId, Arn, Path, CreateDate, Roles: [] } }`

**GetInstanceProfile** -- Get instance profile details.

1. Look up by name; return `NoSuchEntity` if not found
2. Resolve role names to full role records for the response
3. Return `{ InstanceProfile: { ..., Roles: [{ RoleName, RoleId, Arn, ... }] } }`

**DeleteInstanceProfile** -- Delete an instance profile.

1. Validate instance profile has no roles (`DeleteConflict` -- AWS requires removing roles first)
2. Remove from store
3. Return `{}`

**ListInstanceProfiles** -- List all instance profiles.

1. Paginate with path prefix filtering
2. Return `{ InstanceProfiles: [...], IsTruncated, Marker }`

**ListInstanceProfilesForRole** -- List instance profiles containing a specific role.

1. Validate role exists
2. Filter instance profiles by role membership
3. Return `{ InstanceProfiles: [...], IsTruncated, Marker }`

**AddRoleToInstanceProfile** -- Add a role to an instance profile.

1. Validate both exist
2. Validate instance profile does not already contain a role (limit 1 per instance profile)
3. Add role name to `InstanceProfileRecord.roles`
4. Return `{}`

**RemoveRoleFromInstanceProfile** -- Remove a role from an instance profile.

1. Validate both exist
2. Validate role is in the instance profile
3. Remove
4. Return `{}`

#### Phase 2: Policy Versions + Inline Policies (~17 operations)

**CreatePolicyVersion** -- Add a new version to a managed policy.

1. Look up policy by ARN; return `NoSuchEntity` if not found
2. Validate policy has fewer than 5 versions (return `LimitExceeded`)
3. Validate `PolicyDocument` is valid JSON
4. Determine next version ID: `v{max_version_number + 1}`
5. If `SetAsDefault` is true, update the default version
6. Store new `PolicyVersionRecord`
7. Return `{ PolicyVersion: { VersionId, IsDefaultVersion, CreateDate, Document } }`

**GetPolicyVersion** -- Get a specific policy version.

1. Look up policy and version; return `NoSuchEntity` if either not found
2. Return `{ PolicyVersion: { VersionId, IsDefaultVersion, CreateDate, Document } }`

**DeletePolicyVersion** -- Delete a policy version.

1. Cannot delete the default version (return `DeleteConflict`)
2. Remove the version
3. Return `{}`

**ListPolicyVersions** -- List all versions of a policy.

1. Return `{ Versions: [...], IsTruncated, Marker }`

**SetDefaultPolicyVersion** -- Set the default version of a policy.

1. Look up policy and version
2. Clear `is_default_version` on old default
3. Set `is_default_version` on new default
4. Update `ManagedPolicyRecord.default_version_id`
5. Return `{}`

**PutUserPolicy** -- Create or update an inline policy on a user.

1. Validate user exists
2. Validate `PolicyDocument` is valid JSON
3. Store in `UserRecord.inline_policies` (upsert by policy name)
4. Return `{}`

**GetUserPolicy** -- Get an inline policy from a user.

1. Validate user exists
2. Look up policy by name in `UserRecord.inline_policies`; return `NoSuchEntity` if not found
3. Return `{ UserName, PolicyName, PolicyDocument }`
   - Note: `PolicyDocument` is URL-encoded in the response

**DeleteUserPolicy** -- Delete an inline policy from a user.

1. Validate user and policy exist
2. Remove from `UserRecord.inline_policies`
3. Return `{}`

**ListUserPolicies** -- List inline policy names on a user.

1. Validate user exists
2. Return `{ PolicyNames: [...], IsTruncated, Marker }`

**PutRolePolicy / GetRolePolicy / DeleteRolePolicy / ListRolePolicies** -- Same patterns as user inline policies but for roles.

**PutGroupPolicy / GetGroupPolicy / DeleteGroupPolicy / ListGroupPolicies** -- Same patterns as user inline policies but for groups.

#### Phase 3: Advanced (~12 operations)

**TagUser / UntagUser / ListUserTags** -- Tag management for users.

1. `TagUser`: Merge new tags into `UserRecord.tags` (overwrite existing keys). Max 50 tags.
2. `UntagUser`: Remove tags by key from `UserRecord.tags`.
3. `ListUserTags`: Return `{ Tags: [...], IsTruncated, Marker }`.

**TagRole / UntagRole / ListRoleTags** -- Tag management for roles. Same pattern as users.

**CreateServiceLinkedRole** -- Create a service-linked role.

1. Validate `AWSServiceName` (e.g., `elasticmapreduce.amazonaws.com`)
2. Derive role name from service: `AWSServiceRoleFor{ServiceName}`
3. Create role with service-specific trust policy
4. Set `is_service_linked = true`
5. Return `{ Role: { ... } }`

**DeleteServiceLinkedRole** -- Delete a service-linked role (async-style).

1. Generate a deletion task ID
2. Mark role for deletion (or delete immediately for local dev)
3. Return `{ DeletionTaskId }`

**GetServiceLinkedRoleDeletionStatus** -- Check deletion status.

1. Always return `{ Status: SUCCEEDED }` (since we delete immediately in local dev)

**UpdateAssumeRolePolicy** -- Update a role's trust policy.

1. Validate role exists
2. Validate `PolicyDocument` is valid JSON
3. Update `RoleRecord.assume_role_policy_document`
4. Return `{}`

**SimulatePrincipalPolicy / SimulateCustomPolicy** -- Policy simulation (stubs).

1. Accept the request
2. Return a result indicating all actions are `allowed` with `implicitDeny` as the matched statement
3. This is a deliberate simplification; real policy evaluation is a non-goal

**ListEntitiesForPolicy** -- List entities attached to a policy.

1. Look up policy by ARN
2. Iterate all users, roles, groups to find those with this policy attached
3. Return `{ PolicyGroups, PolicyUsers, PolicyRoles, IsTruncated, Marker }`

**GetAccountAuthorizationDetails** -- Get comprehensive account details.

1. Iterate all users, groups, roles, and policies
2. Filter by `Filter` parameter (User, Role, Group, LocalManagedPolicy, AWSManagedPolicy)
3. Return `{ UserDetailList, GroupDetailList, RoleDetailList, Policies, IsTruncated, Marker }`

### 10.3 Validation Rules

| Field                        | Rule                                                          |
| ---------------------------- | ------------------------------------------------------------- |
| UserName                     | 1-64 chars, alphanumeric plus `+=,.@_-`                       |
| RoleName                     | 1-64 chars, alphanumeric plus `+=,.@_-`                       |
| GroupName                    | 1-128 chars, alphanumeric plus `+=,.@_-`                      |
| PolicyName                   | 1-128 chars, alphanumeric plus `+=,.@_-`                      |
| InstanceProfileName          | 1-128 chars, alphanumeric plus `+=,.@_-`                      |
| Path                         | 1-512 chars, must start and end with `/`, regex `(/[!-~]+/)*` |
| PolicyDocument               | Valid JSON, max 6144 chars (managed) or 2048 chars (inline)   |
| AssumeRolePolicyDocument     | Valid JSON, max 2048 chars                                    |
| Description                  | Max 1000 chars                                                |
| MaxSessionDuration           | 3600-43200 seconds                                            |
| Tag key                      | 1-128 chars                                                   |
| Tag value                    | 0-256 chars                                                   |
| Tags per entity              | Max 50                                                        |
| Attached policies per entity | Max 20 (configurable via quotas)                              |
| Inline policies per entity   | Max 0 (unlimited in practice for local dev)                   |
| Policy versions              | Max 5 per managed policy                                      |
| Access keys per user         | Max 2                                                         |
| Roles per instance profile   | Max 1                                                         |
| MaxItems (pagination)        | 1-1000 (default 100)                                          |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// IAM error codes matching the AWS API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IamErrorCode {
    /// Entity (user, role, group, policy, etc.) not found.
    NoSuchEntity,
    /// Entity already exists with this name.
    EntityAlreadyExists,
    /// Cannot delete entity that has subordinate entities.
    DeleteConflict,
    /// Quota or limit exceeded.
    LimitExceeded,
    /// Malformed policy document.
    MalformedPolicyDocument,
    /// Invalid input parameter.
    InvalidInput,
    /// Missing action parameter.
    MissingAction,
    /// Unknown or unsupported action.
    InvalidAction,
    /// Internal service failure.
    ServiceFailure,
    /// Entity temporarily unmodifiable.
    EntityTemporarilyUnmodifiable,
    /// The marker/pagination token is invalid.
    InvalidMarker,
    /// Unmodifiable entity (e.g., AWS managed policy).
    UnmodifiableEntity,
    /// Concurrent modification detected.
    ConcurrentModification,
}
```

### 11.2 Error Mapping

```rust
impl IamErrorCode {
    /// HTTP status code for each error.
    pub fn status_code(&self) -> http::StatusCode {
        match self {
            Self::NoSuchEntity => http::StatusCode::NOT_FOUND,
            Self::EntityAlreadyExists => http::StatusCode::CONFLICT,
            Self::DeleteConflict => http::StatusCode::CONFLICT,
            Self::LimitExceeded => http::StatusCode::CONFLICT,
            Self::MalformedPolicyDocument => http::StatusCode::BAD_REQUEST,
            Self::InvalidInput => http::StatusCode::BAD_REQUEST,
            Self::MissingAction => http::StatusCode::BAD_REQUEST,
            Self::InvalidAction => http::StatusCode::BAD_REQUEST,
            Self::ServiceFailure => http::StatusCode::INTERNAL_SERVER_ERROR,
            Self::EntityTemporarilyUnmodifiable => http::StatusCode::CONFLICT,
            Self::InvalidMarker => http::StatusCode::BAD_REQUEST,
            Self::UnmodifiableEntity => http::StatusCode::BAD_REQUEST,
            Self::ConcurrentModification => http::StatusCode::CONFLICT,
        }
    }

    /// The error code string used in XML responses.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSuchEntity => "NoSuchEntity",
            Self::EntityAlreadyExists => "EntityAlreadyExists",
            Self::DeleteConflict => "DeleteConflict",
            Self::LimitExceeded => "LimitExceeded",
            Self::MalformedPolicyDocument => "MalformedPolicyDocument",
            Self::InvalidInput => "InvalidInput",
            Self::MissingAction => "MissingAction",
            Self::InvalidAction => "InvalidAction",
            Self::ServiceFailure => "ServiceFailure",
            Self::EntityTemporarilyUnmodifiable => "EntityTemporarilyUnmodifiable",
            Self::InvalidMarker => "InvalidMarker",
            Self::UnmodifiableEntity => "UnmodifiableEntity",
            Self::ConcurrentModification => "ConcurrentModification",
        }
    }

    /// Whether this is a Sender or Receiver fault.
    pub fn fault(&self) -> &'static str {
        match self {
            Self::ServiceFailure => "Receiver",
            _ => "Sender",
        }
    }
}
```

### 11.3 Error Response Format

```xml
<ErrorResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
  <Error>
    <Type>Sender</Type>
    <Code>NoSuchEntity</Code>
    <Message>The role with name MyRole cannot be found.</Message>
  </Error>
  <RequestId>7a62c49f-347e-4fc4-9331-6e8eEXAMPLE</RequestId>
</ErrorResponse>
```

This is the same XML error format as SNS, with a different namespace URI.

---

## 12. Server Integration

### 12.1 Feature Gate

IAM support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns", "lambda", "events", "logs", "kms", "kinesis", "secretsmanager", "iam"]
iam = ["dep:rustack-iam-core", "dep:rustack-iam-http"]
```

### 12.2 Gateway Registration

IAM must be registered **before** SNS in the service list, because both match on `Content-Type: application/x-www-form-urlencoded`. The IAM router uses SigV4 service name for primary matching, and the SNS router (which already matches any form-urlencoded POST) acts as the fallback for awsQuery services.

```rust
// In build_services()

// ----- IAM (register BEFORE SNS: both use awsQuery, IAM uses SigV4 service detection) -----
#[cfg(feature = "iam")]
if is_enabled("iam") {
    let iam_config = IamConfig::from_env();
    info!(
        iam_skip_signature_validation = iam_config.skip_signature_validation,
        "initializing IAM service",
    );
    let iam_provider = RustackIam::new(iam_config.clone());
    let iam_handler = RustackIamHandler::new(Arc::new(iam_provider));
    let iam_http_config = build_iam_http_config(&iam_config);
    let iam_service = IamHttpService::new(Arc::new(iam_handler), iam_http_config);
    services.push(Box::new(service::IamServiceRouter::new(iam_service)));
}

// ----- SNS (after IAM, catches remaining awsQuery requests) -----
#[cfg(feature = "sns")]
if is_enabled("sns") {
    // ... existing SNS setup ...
}
```

**Alternative approach (body-based routing):** If SigV4 detection proves unreliable (e.g., unsigned requests in permissive development mode), the gateway can buffer the body for form-urlencoded POSTs and route based on the `Action=` parameter. The `IamServiceRouter::matches()` method would need access to the body, which requires the gateway to buffer and clone it. This is the same approach SNS currently uses implicitly (SNS matches all form-urlencoded POSTs and rejects unknown actions). To support both IAM and SNS without SigV4, the gateway would need to:

1. Buffer the body for `POST /` with `Content-Type: application/x-www-form-urlencoded`
2. Parse `Action=` from the body
3. Route to IAM if Action is in `IAM_ACTIONS`, to SNS if in `SNS_ACTIONS`

Since IAM and SNS have completely disjoint action names, this is unambiguous.

### 12.3 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "available",
        "dynamodb": "available",
        "sqs": "available",
        "ssm": "available",
        "sns": "available",
        "lambda": "available",
        "events": "available",
        "logs": "available",
        "kms": "available",
        "kinesis": "available",
        "secretsmanager": "available",
        "iam": "available"
    },
    "version": "0.4.0"
}
```

### 12.4 Configuration

```rust
/// IAM service configuration.
pub struct IamConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default account ID for ARN generation.
    pub account_id: String,
}

impl IamConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("IAM_SKIP_SIGNATURE_VALIDATION", true),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
        }
    }
}
```

Note: IAM does not use a `default_region` because IAM is a global service. ARNs have no region component.

### 12.5 Environment Variables

| Variable                        | Default        | Purpose                  |
| ------------------------------- | -------------- | ------------------------ |
| `GATEWAY_LISTEN`                | `0.0.0.0:4566` | Bind address (shared)    |
| `IAM_SKIP_SIGNATURE_VALIDATION` | `true`         | Skip SigV4 for IAM       |
| `DEFAULT_ACCOUNT_ID`            | `000000000000` | Default account for ARNs |

### 12.6 Docker Image / GitHub Action

The existing Docker image and GitHub Action gain IAM support automatically when the feature is enabled. The GitHub Action `action.yml` should be updated to list `iam` as a supported service.

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **ARN generation**: Verify correct ARN format for all entity types with various paths
- **ID generation**: Verify prefix correctness and length for all entity types
- **Validation**: Entity name rules, path format, policy document JSON validation, tag limits
- **User CRUD**: Create, get, delete, list with path prefix filtering, update name/path
- **Role CRUD**: Create with trust policy, get, delete, list, update description/session duration
- **Group operations**: Create, delete, add/remove members, list groups for user
- **Managed policy CRUD**: Create with version, get, delete (with and without attachments)
- **Policy versioning**: Create version, set default, delete non-default, enforce 5-version limit
- **Inline policies**: Put, get, delete, list for each entity type (user, role, group)
- **Policy attachment**: Attach, detach, list attached; bidirectional tracking; enforcement of delete-conflict
- **Instance profiles**: Create, delete, add/remove role, enforce 1-role limit
- **Access keys**: Create (verify ID format), delete, list, update status, enforce 2-key limit
- **Tags**: Tag, untag, list tags; enforce 50-tag limit; tag merge semantics
- **Service-linked roles**: Create with proper naming convention, delete with status check
- **Pagination**: Marker-based pagination across all list operations

### 13.2 Integration Tests with aws-sdk-rust

```rust
// tests/integration/iam_tests.rs

#[tokio::test]
#[ignore]
async fn test_should_create_and_get_user() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateUser, GetUser round-trip
}

#[tokio::test]
#[ignore]
async fn test_should_create_role_and_attach_policy() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateRole with trust policy, CreatePolicy, AttachRolePolicy
    // Verify with GetRole and ListAttachedRolePolicies
}

#[tokio::test]
#[ignore]
async fn test_should_manage_instance_profiles() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateInstanceProfile, CreateRole, AddRoleToInstanceProfile
    // GetInstanceProfile, verify role is included
    // RemoveRoleFromInstanceProfile, DeleteInstanceProfile
}

#[tokio::test]
#[ignore]
async fn test_should_manage_policy_versions() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreatePolicy, CreatePolicyVersion (x4), verify 5-version limit
    // SetDefaultPolicyVersion, GetPolicyVersion
    // DeletePolicyVersion
}

#[tokio::test]
#[ignore]
async fn test_should_manage_groups_and_membership() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateGroup, CreateUser, AddUserToGroup
    // GetGroup (verify member list), ListGroupsForUser
    // RemoveUserFromGroup, DeleteGroup
}

#[tokio::test]
#[ignore]
async fn test_should_manage_access_keys() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateUser, CreateAccessKey (verify AKIA prefix)
    // ListAccessKeys, UpdateAccessKey (disable), DeleteAccessKey
}

#[tokio::test]
#[ignore]
async fn test_should_enforce_delete_conflicts() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateRole, AttachRolePolicy, attempt DeleteRole (should fail)
    // DetachRolePolicy, DeleteRole (should succeed)
}

#[tokio::test]
#[ignore]
async fn test_should_manage_inline_policies() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateRole, PutRolePolicy, GetRolePolicy, ListRolePolicies
    // DeleteRolePolicy
}

#[tokio::test]
#[ignore]
async fn test_should_manage_tags() {
    let client = aws_sdk_iam::Client::new(&config);
    // CreateRole, TagRole, ListRoleTags, UntagRole
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Create user
aws iam create-user --user-name testuser --endpoint-url http://localhost:4566

# Create role with trust policy
aws iam create-role --role-name MyLambdaRole \
  --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"lambda.amazonaws.com"},"Action":"sts:AssumeRole"}]}' \
  --endpoint-url http://localhost:4566

# Create policy
aws iam create-policy --policy-name MyPolicy \
  --policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:*","Resource":"*"}]}' \
  --endpoint-url http://localhost:4566

# Attach policy to role
aws iam attach-role-policy --role-name MyLambdaRole \
  --policy-arn arn:aws:iam::000000000000:policy/MyPolicy \
  --endpoint-url http://localhost:4566

# List roles
aws iam list-roles --endpoint-url http://localhost:4566

# Create instance profile
aws iam create-instance-profile --instance-profile-name MyProfile \
  --endpoint-url http://localhost:4566

# Add role to instance profile
aws iam add-role-to-instance-profile --instance-profile-name MyProfile \
  --role-name MyLambdaRole --endpoint-url http://localhost:4566

# Create access key
aws iam create-access-key --user-name testuser --endpoint-url http://localhost:4566

# List access keys
aws iam list-access-keys --user-name testuser --endpoint-url http://localhost:4566

# Put inline policy
aws iam put-role-policy --role-name MyLambdaRole --policy-name InlinePolicy \
  --policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"logs:*","Resource":"*"}]}' \
  --endpoint-url http://localhost:4566

# Tag role
aws iam tag-role --role-name MyLambdaRole \
  --tags Key=Environment,Value=production \
  --endpoint-url http://localhost:4566

# Delete (cleanup: detach first)
aws iam detach-role-policy --role-name MyLambdaRole \
  --policy-arn arn:aws:iam::000000000000:policy/MyPolicy \
  --endpoint-url http://localhost:4566
aws iam delete-role-policy --role-name MyLambdaRole --policy-name InlinePolicy \
  --endpoint-url http://localhost:4566
aws iam remove-role-from-instance-profile --instance-profile-name MyProfile \
  --role-name MyLambdaRole --endpoint-url http://localhost:4566
aws iam delete-instance-profile --instance-profile-name MyProfile \
  --endpoint-url http://localhost:4566
aws iam delete-role --role-name MyLambdaRole --endpoint-url http://localhost:4566
aws iam delete-policy --policy-arn arn:aws:iam::000000000000:policy/MyPolicy \
  --endpoint-url http://localhost:4566
aws iam delete-access-key --user-name testuser --access-key-id AKIA... \
  --endpoint-url http://localhost:4566
aws iam delete-user --user-name testuser --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack IAM Tests

**Location:** `vendors/localstack/tests/aws/services/iam/`
**Coverage:** Comprehensive -- typically 50+ test cases covering:
- User, role, group, policy CRUD
- Policy attachment and detachment
- Instance profiles
- Access key management
- Service-linked roles
- Inline policies
- Tag operations
- Policy versioning
- Delete-conflict enforcement
- Pagination

**How to run:**
```makefile
test-iam-localstack:
	cd vendors/localstack && python -m pytest tests/aws/services/iam/ \
		--endpoint-url=http://localhost:4566 -v
```

#### 13.4.2 Moto IAM Tests

**Source:** https://github.com/getmoto/moto/blob/master/tests/test_iam/
**Coverage:** Moto has the most comprehensive IAM mock implementation, covering all major operations including:
- All user/role/group/policy CRUD
- Policy versioning (5-version limit)
- Inline policies
- Instance profiles
- Access keys
- Service-linked roles
- Tags
- Account authorization details
- Policy simulation

**How to run:**
```makefile
test-iam-moto:
	cd vendors/moto && python -m pytest tests/test_iam/test_iam.py \
		--endpoint-url=http://localhost:4566
```

#### 13.4.3 Terraform AWS Provider Acceptance Tests

**Source:** https://github.com/hashicorp/terraform-provider-aws (tests for `aws_iam_role`, `aws_iam_policy`, `aws_iam_instance_profile`, etc.)
**Coverage:** Tests the full lifecycle of IAM resources as Terraform manages them:
- `aws_iam_role` + `aws_iam_role_policy_attachment`
- `aws_iam_policy` + `aws_iam_policy_attachment`
- `aws_iam_instance_profile`
- `aws_iam_user` + `aws_iam_user_policy`
- `aws_iam_group` + `aws_iam_group_membership`
- `aws_iam_access_key`

**How to run:**
```bash
pip install terraform-local
tflocal init
tflocal apply  # Creates IAM resources against localhost:4566
tflocal destroy
```

**What this validates:** This is the most critical external validation. Terraform is the primary consumer of IAM APIs, and nearly every Terraform module creates IAM roles and policies.

#### 13.4.4 Pulumi

**Source:** https://github.com/pulumi/pulumi-aws
**Coverage:** Similar to Terraform -- creates IAM roles, policies, instance profiles using the AWS SDK.
**How to run:** Configure Pulumi with `AWS_ENDPOINT_URL=http://localhost:4566`.

#### 13.4.5 AWS CDK

**Coverage:** CDK's `aws-iam` constructs (`Role`, `Policy`, `ManagedPolicy`, `User`, `Group`) exercise CreateRole, CreatePolicy, AttachRolePolicy, PutRolePolicy.
**How to run:** Use `cdklocal` (LocalStack's CDK wrapper) to deploy stacks containing IAM resources.

### 13.5 CI Integration

```yaml
# .github/workflows/iam-ci.yml
name: IAM CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-iam-model
      - run: cargo test -p rustack-iam-core
      - run: cargo test -p rustack-iam-http

  integration:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --release
      - run: ./target/release/rustack &
      - run: sleep 2
      - run: |
          # AWS CLI smoke tests
          aws iam create-role --role-name test-role \
            --assume-role-policy-document '{"Version":"2012-10-17","Statement":[]}' \
            --endpoint-url http://localhost:4566
          aws iam get-role --role-name test-role \
            --endpoint-url http://localhost:4566
          aws iam list-roles --endpoint-url http://localhost:4566
      - run: |
          # Python integration tests
          pip install boto3 pytest
          pytest tests/integration/iam/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: Core CRUD (4-5 days)

**Goal:** Basic user, role, policy CRUD and attachment. Enough for `aws iam create-role` + `attach-role-policy` to work, which is the minimum for Terraform Lambda/ECS deployments.

1. **Day 1: Model + Scaffolding**
   - Download IAM Smithy model from AWS API models repository
   - Add `codegen/services/iam.toml` configuration
   - Generate `rustack-iam-model` crate
   - Create `rustack-iam-core` and `rustack-iam-http` crate scaffolding
   - Implement `IamOperation` enum and router

2. **Day 2: Storage Engine + User/Role CRUD**
   - Implement `IamStore` with all DashMaps
   - Implement ID generation (AIDA, AROA, AGPA, ANPA, AIPA prefixes)
   - Implement ARN generation for all entity types
   - Implement validation module (name, path, policy document)
   - Implement `CreateUser`, `GetUser`, `DeleteUser`, `ListUsers`, `UpdateUser`
   - Implement `CreateRole`, `GetRole`, `DeleteRole`, `ListRoles`, `UpdateRole`

3. **Day 3: Policy CRUD + Attachments**
   - Implement `CreatePolicy`, `GetPolicy`, `DeletePolicy`, `ListPolicies`
   - Implement `AttachUserPolicy`, `DetachUserPolicy`, `ListAttachedUserPolicies`
   - Implement `AttachRolePolicy`, `DetachRolePolicy`, `ListAttachedRolePolicies`
   - Implement delete-conflict validation for all entity types

4. **Day 4: Access Keys + Gateway Integration**
   - Implement `CreateAccessKey`, `DeleteAccessKey`, `ListAccessKeys`, `UpdateAccessKey`, `GetAccessKeyLastUsed`
   - Implement HTTP layer: router, service, XML response builder
   - Implement `IamServiceRouter` with SigV4 service detection
   - Integrate into gateway `build_services()` and health endpoint
   - Add feature gate to `apps/rustack/Cargo.toml`

5. **Day 5: Tests + Polish**
   - Unit tests for all Phase 0 operations
   - Integration tests with aws-sdk-rust
   - AWS CLI smoke tests
   - Fix edge cases from manual testing

**Deliverable:** AWS CLI, Terraform basic (`aws_iam_role`, `aws_iam_policy`, `aws_iam_role_policy_attachment`) all work.

### Phase 1: Groups + Instance Profiles (2-3 days)

**Goal:** Group management and instance profiles. Terraform `aws_iam_instance_profile` and `aws_iam_group_membership` work.

6. **Day 6: Groups**
   - Implement `CreateGroup`, `GetGroup`, `DeleteGroup`, `ListGroups`, `UpdateGroup`
   - Implement `AddUserToGroup`, `RemoveUserFromGroup`, `ListGroupsForUser`
   - Implement `AttachGroupPolicy`, `DetachGroupPolicy`, `ListAttachedGroupPolicies`

7. **Day 7: Instance Profiles**
   - Implement `CreateInstanceProfile`, `GetInstanceProfile`, `DeleteInstanceProfile`
   - Implement `ListInstanceProfiles`, `ListInstanceProfilesForRole`
   - Implement `AddRoleToInstanceProfile`, `RemoveRoleFromInstanceProfile`

8. **Day 8: Tests**
   - Unit tests for groups and instance profiles
   - Integration tests
   - Fix edge cases

**Deliverable:** Terraform full (instance profiles, groups), CDK basic, Serverless Framework all work.

### Phase 2: Policy Versions + Inline Policies (2-3 days)

**Goal:** Full policy management. Terraform `aws_iam_role_policy` (inline) and policy versioning work.

9. **Day 9: Policy Versions**
   - Implement `CreatePolicyVersion`, `GetPolicyVersion`, `DeletePolicyVersion`
   - Implement `ListPolicyVersions`, `SetDefaultPolicyVersion`
   - Enforce 5-version limit

10. **Day 10: Inline Policies**
    - Implement `PutUserPolicy`, `GetUserPolicy`, `DeleteUserPolicy`, `ListUserPolicies`
    - Implement `PutRolePolicy`, `GetRolePolicy`, `DeleteRolePolicy`, `ListRolePolicies`
    - Implement `PutGroupPolicy`, `GetGroupPolicy`, `DeleteGroupPolicy`, `ListGroupPolicies`

**Deliverable:** All policy management works. CDK full, Serverless full.

### Phase 3: Advanced + CI (2-3 days)

**Goal:** Tags, service-linked roles, simulation stubs, CI pipeline.

11. **Day 11: Tags + Service-Linked Roles**
    - Implement `TagUser`, `UntagUser`, `ListUserTags`
    - Implement `TagRole`, `UntagRole`, `ListRoleTags`
    - Implement `CreateServiceLinkedRole`, `DeleteServiceLinkedRole`, `GetServiceLinkedRoleDeletionStatus`
    - Implement `UpdateAssumeRolePolicy`

12. **Day 12: Advanced + Stubs**
    - Implement `SimulatePrincipalPolicy`, `SimulateCustomPolicy` (stubs)
    - Implement `ListEntitiesForPolicy`
    - Implement `GetAccountAuthorizationDetails`

13. **Day 13: CI + Polish**
    - CI workflow for IAM
    - Update Docker image, GitHub Action
    - Run LocalStack test suite subset, document pass/fail
    - Final cleanup and documentation

**Deliverable:** All ~60 operations implemented, CI green, Docker image updated.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk                                                  | Likelihood | Impact | Mitigation                                                                                                                                                                                                                                             |
| ----------------------------------------------------- | ---------- | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Gateway routing collision between IAM and SNS         | Medium     | High   | Primary: SigV4 service name extraction from Authorization header. Fallback: disjoint Action name sets. Both strategies are reliable and complementary. Test with unsigned requests to verify fallback path works.                                      |
| URL-encoded policy documents in XML responses         | Medium     | Medium | IAM returns `AssumeRolePolicyDocument` and inline policy documents as URL-encoded strings in XML. Must match AWS behavior exactly or SDK parsing breaks. Test with aws-sdk-rust and AWS CLI to verify round-trip fidelity.                             |
| Delete-conflict enforcement too strict or too lenient | Medium     | Medium | AWS enforces strict ordering: must detach all policies and remove all memberships before deleting an entity. Test all permutations: delete user with attached policies, with group memberships, with inline policies, with access keys.                |
| Policy version limit enforcement (max 5)              | Low        | Medium | Must correctly enforce the 5-version limit and prevent deletion of the default version. Edge case: create 5 versions, try to create 6th (should fail), delete non-default, create new (should succeed).                                                |
| Pagination marker format differs from AWS             | Low        | Low    | AWS uses opaque marker strings. We can use simple offset-based markers encoded as strings. AWS SDKs treat markers as opaque, so the format does not matter as long as it round-trips correctly.                                                        |
| Instance profile 1-role limit not enforced            | Low        | Medium | AWS allows at most 1 role per instance profile (despite the API accepting a list). Must enforce this limit.                                                                                                                                            |
| Access key ID format not realistic enough             | Low        | Low    | Some tools may validate that access key IDs match the `AKIA[A-Z0-9]{16}` pattern. Our generation matches this format.                                                                                                                                  |
| Form parameter parsing for nested types               | Medium     | Medium | IAM uses `Tags.member.N.Key=...&Tags.member.N.Value=...` encoding for lists. Must handle the `member.N` convention correctly. Reuse SNS's form parsing patterns.                                                                                       |
| XML response field ordering                           | Low        | Low    | Some XML parsers may be sensitive to element ordering. Match AWS response element ordering from the Smithy model. Test with multiple SDK versions.                                                                                                     |
| Concurrent modifications to shared entities           | Low        | Low    | Two concurrent requests modifying the same role/user could race. DashMap's per-entry locking handles most cases. For cross-entity operations (AttachRolePolicy modifies both role and policy), acquire locks in consistent order to prevent deadlocks. |

### 15.2 Dependencies

- `rustack-core` -- no changes needed (IAM is global, but the global/regional distinction is handled in IAM core, not in rustack-core)
- `rustack-auth` -- no changes needed (SigV4 with service=`iam`)
- `dashmap` -- already in workspace
- `serde_json` -- for policy document JSON validation (already in workspace)
- `serde_urlencoded` -- for form parameter parsing (already in workspace for SNS)
- `rand` -- for ID and access key generation (already in workspace)
- `chrono` -- for ISO 8601 timestamp formatting (already in workspace)
- `uuid` -- for request IDs (already in workspace)
- New Smithy model: `codegen/smithy-model/iam.json` must be downloaded from AWS API models

### 15.3 Decision Log

| Decision                                                    | Rationale                                                                                                                                                                                                                     |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Use SigV4 service name for gateway routing (primary)        | Most reliable signal: every AWS SDK includes the service name in the SigV4 credential scope. No body buffering needed. Completely unambiguous.                                                                                |
| Disjoint Action names as fallback routing                   | For unsigned requests (local dev with SigV4 disabled), we can still route correctly because IAM and SNS have zero overlap in Action names. This is a defense-in-depth approach.                                               |
| Global store (account_id only, no region)                   | IAM is a global service in AWS. There is no per-region IAM. Simplifies storage: single flat namespace per account.                                                                                                            |
| Store policy documents as JSON strings without parsing      | No policy enforcement engine. Storing as strings is sufficient for round-trip fidelity. Validate JSON syntax only to catch obvious errors.                                                                                    |
| 6 separate DashMaps (not a single DashMap with enum values) | Each entity type has different fields and access patterns. Separate maps provide type safety and avoid enum dispatching on every access.                                                                                      |
| Bidirectional relationship tracking for policy attachments  | Need to efficiently answer both "what policies does this role have?" (role side) and "what entities use this policy?" (policy side: for attachment_count and ListEntitiesForPolicy).                                          |
| Access key secret returned only on creation                 | Matches AWS behavior exactly. The secret access key is stored in the AccessKeyRecord for internal use but never returned after the initial CreateAccessKey response.                                                          |
| Register IAM router before SNS in gateway                   | IAM uses SigV4-based matching which is more specific than SNS's Content-Type-based matching. Registering IAM first means IAM claims its requests before SNS's catch-all awsQuery matching.                                    |
| ISO 8601 timestamps (not epoch seconds)                     | IAM uses ISO 8601 format (`2026-03-19T12:00:00Z`) in XML responses, unlike Secrets Manager and SSM which use epoch seconds. This is protocol-specific.                                                                        |
| No STS integration                                          | STS (GetCallerIdentity, AssumeRole) is a separate service with its own endpoint. IAM and STS are logically related but architecturally separate. STS can be added later as `rustack-sts-{model,core,http}`.                 |
| SimulatePrincipalPolicy returns "allowed" for everything    | Building a real IAM policy evaluation engine is a massive undertaking (AWS's policy language supports conditions, wildcards, NotAction, etc.). Returning "allowed" for all simulations is the pragmatic choice for local dev. |
