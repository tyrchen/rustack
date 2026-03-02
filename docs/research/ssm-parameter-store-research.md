# SSM Parameter Store API Comprehensive Research

**Date:** 2026-03-02
**Purpose:** Comprehensive analysis of the AWS SSM Parameter Store API surface, protocol, data model, Smithy model, test suites, and what a Rust-based local SSM Parameter Store implementation would need.

---

## Table of Contents

1. [Protocol: AWS JSON 1.1 over HTTP](#1-protocol-aws-json-11-over-http)
2. [Smithy Model Location and Service Definition](#2-smithy-model-location-and-service-definition)
3. [SSM Service Scope: Full vs. Parameter Store](#3-ssm-service-scope-full-vs-parameter-store)
4. [Parameter Store Data Model](#4-parameter-store-data-model)
5. [Parameter Store Operations (14 Operations)](#5-parameter-store-operations-14-operations)
6. [Error Handling](#6-error-handling)
7. [Constraints and Limits](#7-constraints-and-limits)
8. [Third-Party Test Suites](#8-third-party-test-suites)
9. [Tools That Use SSM Parameter Store](#9-tools-that-use-ssm-parameter-store)
10. [Implementation Priority Matrix](#10-implementation-priority-matrix)
11. [Architecture Considerations for RustStack](#11-architecture-considerations-for-ruststack)

---

## 1. Protocol: AWS JSON 1.1 over HTTP

SSM uses the `@awsJson1_1` Smithy protocol, which is very similar to DynamoDB's `@awsJson1_0` but with a slightly different content type.

### 1.1 Request Format

All SSM requests are **HTTP POST** to the root path (`/`). Operation dispatch is done via the `X-Amz-Target` header.

```http
POST / HTTP/1.1
Host: ssm.<region>.amazonaws.com
Content-Type: application/x-amz-json-1.1
X-Amz-Target: AmazonSSM.GetParameter
Authorization: AWS4-HMAC-SHA256 Credential=<...>, SignedHeaders=<...>, Signature=<...>
X-Amz-Date: 20260302T120000Z
Content-Length: <n>

{
    "Name": "MyGitHubPassword",
    "WithDecryption": true
}
```

**Key protocol details:**

| Property | Value |
|----------|-------|
| HTTP Method | `POST` (always) |
| URL Path | `/` (always) |
| Content-Type | `application/x-amz-json-1.1` (always) |
| X-Amz-Target | `AmazonSSM.<OperationName>` |
| API Version | `2014-11-06` |
| Auth | AWS Signature Version 4 |
| Body | JSON (UTF-8 encoded) |
| Empty input | `{}` (empty JSON object) |
| Signed Headers | `content-type`, `host`, `x-amz-date`, `x-amz-target` |

### 1.2 Response Format

```http
HTTP/1.1 200 OK
x-amzn-RequestId: <RequestId>
Content-Type: application/x-amz-json-1.1
Content-Length: <n>

{
    "Parameter": {
        "ARN": "arn:aws:ssm:us-east-2:111122223333:parameter/MyGitHubPassword",
        "DataType": "text",
        "LastModifiedDate": 1582657288.8,
        "Name": "MyGitHubPassword",
        "Type": "SecureString",
        "Value": "AYA39c3b3042cd2aEXAMPLE...",
        "Version": 3
    }
}
```

### 1.3 Error Response Format

Errors are returned with the error type in the `__type` field:

```http
HTTP/1.1 400 Bad Request
Content-Type: application/x-amz-json-1.1

{
    "__type": "ParameterNotFound",
    "message": "The parameter could not be found. Verify the name and try again."
}
```

Alternatively, the error type may appear in the `X-Amzn-Errortype` response header or the `code` field in the body. Clients strip everything after `:` or before `#` in the error type to get the shape name.

### 1.4 Differences from DynamoDB (awsJson1.0)

| Property | DynamoDB (1.0) | SSM (1.1) |
|----------|---------------|-----------|
| Content-Type | `application/x-amz-json-1.0` | `application/x-amz-json-1.1` |
| X-Amz-Target prefix | `DynamoDB_20120810` | `AmazonSSM` |
| Timestamp format | Epoch seconds (double) | Epoch seconds (double) |
| Everything else | Identical | Identical |

The wire format is practically identical. The same JSON serialization/deserialization infrastructure used for DynamoDB can be reused for SSM.

### 1.5 Shape Serialization Rules

Same as awsJson1.0:

| Smithy Type | JSON Representation |
|-------------|---------------------|
| `blob` | Base64-encoded string |
| `boolean` | JSON boolean |
| `byte/short/integer/long` | JSON number |
| `float/double` | JSON number (special: `"NaN"`, `"Infinity"`, `"-Infinity"`) |
| `string` | JSON string |
| `timestamp` | Unix epoch seconds as JSON number (double) |
| `list` | JSON array |
| `map` | JSON object |
| `structure` | JSON object with member-named properties |
| `union` | JSON object with single non-null member |

---

## 2. Smithy Model Location and Service Definition

### 2.1 Official Smithy Model

The official Smithy model for SSM is hosted in the AWS API Models repository:

- **Repository:** [aws/api-models-aws](https://github.com/aws/api-models-aws)
- **Path:** `models/ssm/service/2014-11-06/ssm-2014-11-06.json`
- **Format:** Smithy JSON AST (machine-readable)
- **Updates:** Daily to Maven Central and GitHub

### 2.2 Service Metadata

| Property | Value |
|----------|-------|
| Service Name | `AmazonSSM` |
| SDK ID | `SSM` |
| API Version | `2014-11-06` |
| Protocol | `@awsJson1_1` |
| Signing Name | `ssm` |
| Endpoint Prefix | `ssm` |
| Endpoint Format | `ssm.<region>.amazonaws.com` |

### 2.3 Code Generation

The same Smithy codegen infrastructure used for DynamoDB in RustStack can be reused for SSM, since both use the AWS JSON protocol family. The primary change is:
- Different Content-Type header (`1.1` vs `1.0`)
- Different X-Amz-Target prefix (`AmazonSSM` vs `DynamoDB_20120810`)

For RustStack, we should **NOT** generate from the full SSM model (146 operations). Instead, we should extract only the Parameter Store subset (~14 operations) and generate types from that.

---

## 3. SSM Service Scope: Full vs. Parameter Store

AWS Systems Manager is a **massive** service with 146 API operations spanning many features. For RustStack, we only care about **Parameter Store**.

### 3.1 Full SSM Features (NOT implementing)

| Feature | Operation Count | Examples |
|---------|----------------|----------|
| Run Command | ~7 | SendCommand, ListCommands, GetCommandInvocation |
| Automation | ~8 | StartAutomationExecution, DescribeAutomationExecutions |
| State Manager | ~8 | CreateAssociation, ListAssociations |
| Patch Manager | ~15 | CreatePatchBaseline, DescribePatchGroups |
| Maintenance Windows | ~18 | CreateMaintenanceWindow, RegisterTaskWithMaintenanceWindow |
| Inventory | ~5 | PutInventory, GetInventory |
| Documents (SSM Docs) | ~10 | CreateDocument, ListDocuments |
| OpsCenter/OpsItems | ~8 | CreateOpsItem, DescribeOpsItems |
| Session Manager | ~4 | StartSession, TerminateSession |
| Managed Instances | ~5 | DeregisterManagedInstance, DescribeInstanceInformation |
| Compliance | ~4 | ListComplianceItems, PutComplianceItems |
| Resource Data Sync | ~3 | CreateResourceDataSync |
| Service Settings | ~3 | GetServiceSetting, UpdateServiceSetting |
| Resource Policies | ~3 | PutResourcePolicy, GetResourcePolicies |
| Node Management | ~2 | ListNodes, ListNodesSummary |

### 3.2 Parameter Store Operations (IMPLEMENTING)

The Parameter Store operations are the subset we need:

| # | Operation | Description |
|---|-----------|-------------|
| 1 | `PutParameter` | Create or update a parameter |
| 2 | `GetParameter` | Get a single parameter by name/ARN |
| 3 | `GetParameters` | Get multiple parameters by name (batch, up to 10) |
| 4 | `GetParametersByPath` | Get all parameters under a hierarchy path |
| 5 | `GetParameterHistory` | Get version history of a parameter |
| 6 | `DescribeParameters` | List/search parameter metadata (with filters) |
| 7 | `DeleteParameter` | Delete a single parameter |
| 8 | `DeleteParameters` | Delete multiple parameters (batch, up to 10) |
| 9 | `LabelParameterVersion` | Attach labels to a parameter version |
| 10 | `UnlabelParameterVersion` | Remove labels from a parameter version |
| 11 | `AddTagsToResource` | Add tags to a parameter (shared with other SSM resources) |
| 12 | `RemoveTagsFromResource` | Remove tags from a parameter (shared with other SSM resources) |
| 13 | `ListTagsForResource` | List tags on a parameter (shared with other SSM resources) |

The tag operations (`AddTagsToResource`, `RemoveTagsFromResource`, `ListTagsForResource`) are shared across all SSM resource types. For RustStack, we only need to support `ResourceType: "Parameter"`.

**Optionally** (for completeness):
| # | Operation | Description |
|---|-----------|-------------|
| 14 | `GetServiceSetting` | Get Parameter Store service settings (throughput, tier) |
| 15 | `UpdateServiceSetting` | Update service settings |
| 16 | `ResetServiceSetting` | Reset service settings to defaults |

---

## 4. Parameter Store Data Model

### 4.1 Parameter Types

| Type | Description | Encryption |
|------|-------------|------------|
| `String` | Plain text value, up to 4 KB (Standard) / 8 KB (Advanced) | No |
| `StringList` | Comma-separated list of values | No |
| `SecureString` | Encrypted value, requires KMS key | Yes (KMS) |

### 4.2 Parameter Tiers

| Tier | Max Count/Region | Max Size | Policies | Cost |
|------|-----------------|----------|----------|------|
| Standard | 10,000 | 4 KB | No | Free |
| Advanced | 100,000 | 8 KB | Yes | Charged |
| Intelligent-Tiering | Auto-switches | Auto | Auto | Variable |

For RustStack local dev, tier distinction is informational only -- no real AWS billing.

### 4.3 Parameter Object (API response)

```json
{
    "ARN": "arn:aws:ssm:us-east-2:111122223333:parameter/my/param",
    "DataType": "text",
    "LastModifiedDate": 1582657288.8,
    "Name": "/my/param",
    "Selector": "/my/param:3",
    "SourceResult": "string",
    "Type": "String",
    "Value": "my-value",
    "Version": 3
}
```

### 4.4 ParameterMetadata Object (DescribeParameters response)

```json
{
    "AllowedPattern": "^[a-z]+$",
    "ARN": "arn:aws:ssm:us-east-2:111122223333:parameter/my/param",
    "DataType": "text",
    "Description": "My parameter description",
    "KeyId": "alias/aws/ssm",
    "LastModifiedDate": 1582657288.8,
    "LastModifiedUser": "arn:aws:iam::111122223333:user/Admin",
    "Name": "/my/param",
    "Policies": [],
    "Tier": "Standard",
    "Type": "String",
    "Version": 3
}
```

### 4.5 ParameterHistory Object (GetParameterHistory response)

```json
{
    "AllowedPattern": "string",
    "DataType": "text",
    "Description": "string",
    "KeyId": "string",
    "Labels": ["prod", "v1.0"],
    "LastModifiedDate": 1582657288.8,
    "LastModifiedUser": "arn:aws:iam::111122223333:user/Admin",
    "Name": "/my/param",
    "Policies": [
        {
            "PolicyStatus": "Pending",
            "PolicyType": "Expiration",
            "PolicyText": "{...}"
        }
    ],
    "Tier": "Standard",
    "Type": "String",
    "Value": "old-value",
    "Version": 1
}
```

### 4.6 Hierarchical Parameters (Paths)

Parameters support path-based hierarchy using forward slashes:

- Max depth: 15 levels
- Must start with `/` for hierarchy paths
- Example: `/myapp/production/database/connection-string`
- `GetParametersByPath` can recurse through the hierarchy

### 4.7 Parameter Versioning

- Each `PutParameter` with `Overwrite: true` increments the version
- Parameter Store retains the **100 most recent versions**
- When the 100-version limit is reached, the oldest version is deleted
- **Exception:** If the oldest version has a label attached, deletion is blocked (`ParameterMaxVersionLimitExceeded` error)
- Specific versions can be referenced via `name:version` selector syntax
- Specific labels can be referenced via `name:label` selector syntax

### 4.8 Labels

- Up to 10 labels per parameter version
- Max 100 characters per label
- Labels are unique per parameter (cannot attach same label to two versions)
- Labels can be moved between versions
- Cannot start with `aws`, `ssm`, or a number
- Valid characters: letters (case-sensitive), numbers, `.`, `-`, `_`

### 4.9 Tags

Tags are key-value metadata attached to the parameter resource (not a specific version):

- Max 50 tags per parameter
- Tag key: 1-128 characters
- Tag value: 0-256 characters
- Tags can be specified during `PutParameter` (creation only) or via `AddTagsToResource`

### 4.10 Parameter Name Validation

- Length: 1-1011 characters (user-specified portion; internal max is 2048)
- Cannot contain spaces
- Valid characters: `a-zA-Z0-9_.-/`
- Cannot be prefixed with `aws` or `ssm` (case-insensitive)
- Case-sensitive (e.g., `/Prod/DB` and `/prod/db` are different)

### 4.11 ARN Format

```
arn:aws:ssm:<region>:<account-id>:parameter/<parameter-name>
```

Example: `arn:aws:ssm:us-east-2:111122223333:parameter/myapp/prod/db-password`

Note: For parameter names starting with `/`, the ARN becomes `arn:aws:ssm:...:parameter/myapp/...` (a single slash between `parameter` and the name).

### 4.12 DataType

| DataType | Description |
|----------|-------------|
| `text` | Default. Plain text value. |
| `aws:ec2:image` | Validates the value is a valid AMI ID. Validation is asynchronous. |
| `aws:ssm:integration` | Integration data type. |

For RustStack, `text` is the only DataType we need to fully support.

### 4.13 Public Parameters

AWS publishes public parameters (e.g., latest AMI IDs) under paths like `/aws/service/ami-amazon-linux-latest/`. These are read-only, AWS-managed parameters available in all regions. For RustStack local dev, we do **not** need to replicate public parameters.

---

## 5. Parameter Store Operations (14 Operations)

### 5.1 PutParameter

Creates or updates a parameter.

**X-Amz-Target:** `AmazonSSM.PutParameter`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password",
    "Value": "s3cr3t",
    "Type": "SecureString",
    "Description": "Database password for production",
    "KeyId": "alias/my-key",
    "Overwrite": true,
    "AllowedPattern": "^.{8,}$",
    "DataType": "text",
    "Tier": "Standard",
    "Policies": "[{\"Type\":\"Expiration\",\"Version\":\"1.0\",\"Attributes\":{\"Timestamp\":\"2025-12-31T00:00:00Z\"}}]",
    "Tags": [
        {"Key": "Environment", "Value": "Production"}
    ]
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Name | String | Yes | 1-2048 chars, `a-zA-Z0-9_.-/` |
| Value | String | Yes | 4 KB (Standard) / 8 KB (Advanced) |
| Type | String | Yes (create) | `String`, `StringList`, `SecureString` |
| Description | String | No | 0-1024 chars |
| KeyId | String | No | KMS key ID for SecureString |
| Overwrite | Boolean | No | Default: false |
| AllowedPattern | String | No | Regex to validate Value |
| DataType | String | No | `text`, `aws:ec2:image`, `aws:ssm:integration` |
| Tier | String | No | `Standard`, `Advanced`, `Intelligent-Tiering` |
| Policies | String | No | JSON array, Advanced tier only |
| Tags | Array[Tag] | No | Only on creation, max 1000 |

**Response:**
```json
{
    "Version": 2,
    "Tier": "Standard"
}
```

### 5.2 GetParameter

Get a single parameter by name, ARN, version selector, or label selector.

**X-Amz-Target:** `AmazonSSM.GetParameter`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password",
    "WithDecryption": true
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Name | String | Yes | Name, ARN, `name:version`, or `name:label` |
| WithDecryption | Boolean | No | Decrypt SecureString values |

**Response:**
```json
{
    "Parameter": {
        "ARN": "arn:aws:ssm:us-east-2:111122223333:parameter/myapp/prod/db-password",
        "DataType": "text",
        "LastModifiedDate": 1582657288.8,
        "Name": "/myapp/prod/db-password",
        "Selector": "/myapp/prod/db-password:2",
        "Type": "SecureString",
        "Value": "s3cr3t",
        "Version": 2
    }
}
```

### 5.3 GetParameters

Batch get up to 10 parameters by name.

**X-Amz-Target:** `AmazonSSM.GetParameters`

**Request:**
```json
{
    "Names": ["/myapp/prod/db-password", "/myapp/prod/db-host", "/myapp/prod/nonexistent"],
    "WithDecryption": true
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Names | Array[String] | Yes | 1-10 names/ARNs |
| WithDecryption | Boolean | No | Decrypt SecureString values |

**Response:**
```json
{
    "Parameters": [
        {"ARN": "...", "Name": "/myapp/prod/db-password", "Type": "SecureString", "Value": "s3cr3t", "Version": 2},
        {"ARN": "...", "Name": "/myapp/prod/db-host", "Type": "String", "Value": "db.example.com", "Version": 1}
    ],
    "InvalidParameters": ["/myapp/prod/nonexistent"]
}
```

### 5.4 GetParametersByPath

Retrieve all parameters under a hierarchy path, with optional recursion and filtering.

**X-Amz-Target:** `AmazonSSM.GetParametersByPath`

**Request:**
```json
{
    "Path": "/myapp/prod/",
    "Recursive": true,
    "WithDecryption": true,
    "MaxResults": 10,
    "NextToken": "...",
    "ParameterFilters": [
        {"Key": "Type", "Values": ["SecureString"]},
        {"Key": "Label", "Values": ["production"]}
    ]
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Path | String | Yes | Hierarchy path starting with `/` |
| Recursive | Boolean | No | Default: false (one level only) |
| WithDecryption | Boolean | No | Decrypt SecureString values |
| MaxResults | Integer | No | 1-10 per page |
| NextToken | String | No | Pagination token |
| ParameterFilters | Array | No | Filter by `Type`, `KeyId`, `Label` only |

**Response:**
```json
{
    "Parameters": [...],
    "NextToken": "..."
}
```

**Note:** Filters supported for `GetParametersByPath` are limited: only `Type`, `KeyId`, `Label`. Does NOT support `tag`, `DataType`, `Name`, `Path`, `Tier`.

### 5.5 GetParameterHistory

Get version history of a parameter.

**X-Amz-Target:** `AmazonSSM.GetParameterHistory`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password",
    "WithDecryption": true,
    "MaxResults": 50,
    "NextToken": "..."
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Name | String | Yes | Parameter name or ARN |
| WithDecryption | Boolean | No | Decrypt SecureString values |
| MaxResults | Integer | No | 1-50 |
| NextToken | String | No | Pagination token |

**Response:** Array of `ParameterHistory` objects (includes Labels, Policies, Description, etc.)

### 5.6 DescribeParameters

List/search parameter metadata with rich filtering.

**X-Amz-Target:** `AmazonSSM.DescribeParameters`

**Request:**
```json
{
    "ParameterFilters": [
        {"Key": "Name", "Option": "BeginsWith", "Values": ["/myapp/"]},
        {"Key": "Type", "Values": ["String"]}
    ],
    "MaxResults": 50,
    "NextToken": "..."
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| ParameterFilters | Array | No | Filter by Name, Type, KeyId, Path, Tier, tag, DataType |
| Filters | Array | No | **Deprecated.** Legacy filter format |
| MaxResults | Integer | No | 1-50 |
| NextToken | String | No | Pagination token |
| Shared | Boolean | No | List parameters shared via AWS RAM |

**Filter Key options:** `Name` (BeginsWith, Equals), `Type` (Equals), `KeyId` (Equals), `Path` (Recursive, OneLevel), `Tier` (Equals), `tag:<key>` (Equals), `DataType` (Equals)

**Response:** Array of `ParameterMetadata` objects (no values, includes metadata like Description, Tier, Policies).

### 5.7 DeleteParameter

Delete a single parameter.

**X-Amz-Target:** `AmazonSSM.DeleteParameter`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password"
}
```

**Response:** `{}` (empty)

**Errors:** `ParameterNotFound` (400), `InternalServerError` (500)

### 5.8 DeleteParameters

Batch delete up to 10 parameters.

**X-Amz-Target:** `AmazonSSM.DeleteParameters`

**Request:**
```json
{
    "Names": ["/myapp/prod/db-password", "/myapp/prod/db-host"]
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Names | Array[String] | Yes | 1-10 parameter names (not ARNs) |

**Response:**
```json
{
    "DeletedParameters": ["/myapp/prod/db-password", "/myapp/prod/db-host"],
    "InvalidParameters": []
}
```

### 5.9 LabelParameterVersion

Attach labels to a parameter version.

**X-Amz-Target:** `AmazonSSM.LabelParameterVersion`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password",
    "ParameterVersion": 3,
    "Labels": ["production", "v1.0"]
}
```

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| Name | String | Yes | Parameter name (not ARN) |
| ParameterVersion | Long | No | If omitted, labels the latest version |
| Labels | Array[String] | Yes | 1-10 labels, max 100 chars each |

**Response:**
```json
{
    "InvalidLabels": [],
    "ParameterVersion": 3
}
```

### 5.10 UnlabelParameterVersion

Remove labels from a parameter version.

**X-Amz-Target:** `AmazonSSM.UnlabelParameterVersion`

**Request:**
```json
{
    "Name": "/myapp/prod/db-password",
    "ParameterVersion": 3,
    "Labels": ["v1.0"]
}
```

**Response:**
```json
{
    "InvalidLabels": [],
    "RemovedLabels": ["v1.0"]
}
```

### 5.11 AddTagsToResource

Add tags to a parameter (shared operation).

**X-Amz-Target:** `AmazonSSM.AddTagsToResource`

**Request:**
```json
{
    "ResourceType": "Parameter",
    "ResourceId": "/myapp/prod/db-password",
    "Tags": [
        {"Key": "Environment", "Value": "Production"},
        {"Key": "Owner", "Value": "DevOps"}
    ]
}
```

**Response:** `{}` (empty)

### 5.12 RemoveTagsFromResource

Remove tags from a parameter.

**X-Amz-Target:** `AmazonSSM.RemoveTagsFromResource`

**Request:**
```json
{
    "ResourceType": "Parameter",
    "ResourceId": "/myapp/prod/db-password",
    "TagKeys": ["Environment"]
}
```

**Response:** `{}` (empty)

### 5.13 ListTagsForResource

List tags on a parameter.

**X-Amz-Target:** `AmazonSSM.ListTagsForResource`

**Request:**
```json
{
    "ResourceType": "Parameter",
    "ResourceId": "/myapp/prod/db-password"
}
```

**Response:**
```json
{
    "TagList": [
        {"Key": "Owner", "Value": "DevOps"}
    ]
}
```

---

## 6. Error Handling

### 6.1 Common Error Types

| Error Type | HTTP Status | Operations | Description |
|------------|-------------|------------|-------------|
| `InternalServerError` | 500 | All | Server-side error |
| `ParameterNotFound` | 400 | Get, Delete, History, Label, Unlabel | Parameter does not exist |
| `ParameterAlreadyExists` | 400 | PutParameter | Name exists and Overwrite=false |
| `ParameterLimitExceeded` | 400 | PutParameter | Max parameters for account/region |
| `ParameterMaxVersionLimitExceeded` | 400 | PutParameter | 100 versions reached, oldest has label |
| `ParameterVersionNotFound` | 400 | GetParameter, Label, Unlabel | Specified version does not exist |
| `ParameterVersionLabelLimitExceeded` | 400 | LabelParameterVersion | 10 labels per version limit |
| `HierarchyLevelLimitExceededException` | 400 | PutParameter | More than 15 levels |
| `HierarchyTypeMismatchException` | 400 | PutParameter | Cannot change type in hierarchy |
| `InvalidKeyId` | 400 | GetParameter, GetParameters, GetByPath, History | Invalid KMS key |
| `InvalidFilterKey` | 400 | DescribeParameters, GetByPath | Invalid filter key |
| `InvalidFilterOption` | 400 | DescribeParameters, GetByPath | Invalid filter option |
| `InvalidFilterValue` | 400 | DescribeParameters, GetByPath | Invalid filter value |
| `InvalidNextToken` | 400 | DescribeParameters, GetByPath, History | Invalid pagination token |
| `InvalidAllowedPatternException` | 400 | PutParameter | Value does not match AllowedPattern |
| `ParameterPatternMismatchException` | 400 | PutParameter | Invalid parameter name |
| `UnsupportedParameterType` | 400 | PutParameter | Invalid parameter type |
| `TooManyUpdates` | 400 | PutParameter, Label, Unlabel, Tags | Concurrent update conflict |
| `TooManyTagsError` | 400 | AddTagsToResource | Max 50 tags exceeded |
| `InvalidResourceId` | 400 | Tag operations | Invalid resource ID |
| `InvalidResourceType` | 400 | Tag operations | Invalid resource type |
| `InvalidPolicyTypeException` | 400 | PutParameter | Invalid policy type |
| `InvalidPolicyAttributeException` | 400 | PutParameter | Invalid policy attribute |
| `IncompatiblePolicyException` | 400 | PutParameter | Conflicting policies |
| `PoliciesLimitExceededException` | 400 | PutParameter | More than 10 policies |

### 6.2 Error Response Format

```json
{
    "__type": "ParameterNotFound",
    "message": "The parameter could not be found. Verify the name and try again."
}
```

---

## 7. Constraints and Limits

### 7.1 Storage Limits

| Resource | Standard Tier | Advanced Tier |
|----------|--------------|---------------|
| Max parameters per region | 10,000 | 100,000 |
| Max parameter value size | 4 KB | 8 KB |
| Max parameter name length | 1,011 chars (user) / 2,048 chars (internal) | Same |
| Max hierarchy depth | 15 levels | 15 levels |
| Max versions retained | 100 | 100 |
| Max labels per version | 10 | 10 |
| Max tags per parameter | 50 | 50 |
| Max policies per parameter | N/A | 10 |

### 7.2 API Limits

| Constraint | Value |
|------------|-------|
| GetParameters batch size | 1-10 |
| DeleteParameters batch size | 1-10 |
| GetParametersByPath MaxResults | 1-10 |
| DescribeParameters MaxResults | 1-50 |
| GetParameterHistory MaxResults | 1-50 |
| LabelParameterVersion labels | 1-10 |
| Label max length | 100 characters |
| Description max length | 1,024 characters |
| AllowedPattern max length | 1,024 characters |

### 7.3 Throughput Limits

| Setting | Standard | High-Throughput |
|---------|----------|-----------------|
| API transactions/second | 40 | 1,000 |
| GetParameter | 40 TPS | 1,000 TPS |
| GetParametersByPath | 40 TPS | 1,000 TPS |
| PutParameter | 40 TPS | 1,000 TPS |

For RustStack local dev, throughput limits are irrelevant.

---

## 8. Third-Party Test Suites

### 8.1 moto (Python) -- Best Reference Implementation

**Repository:** [getmoto/moto](https://github.com/getmoto/moto)
**Test files:**
- `tests/test_ssm/test_ssm.py` -- Main Parameter Store tests (66+ test functions)
- `tests/test_ssm/test_ssm_parameterstore.py` -- ParameterDict internal tests
- `tests/test_ssm/test_ssm_cloudformation.py` -- CloudFormation integration
- `tests/test_ssm/test_ssm_defaults.py` -- Default/public parameters
- `tests/test_ssm/test_ssm_default_amis.py` -- AMI public parameters
- `tests/test_ssm/test_ssm_doc_permissions.py` -- Document permissions
- `tests/test_ssm/test_ssm_docs.py` -- SSM Documents
- `tests/test_ssm/test_ssm_ec2_integration.py` -- EC2 integration
- `tests/test_ssm/test_ssm_maintenance_windows.py` -- Maintenance windows
- `tests/test_ssm/test_ssm_patch_baseline.py` -- Patch baselines
- `tests/test_ssm/test_ssm_secretsmanager.py` -- Secrets Manager integration
- `tests/test_ssm/test_ssm_utils.py` -- Utilities

**Key Parameter Store test functions in `test_ssm.py` (66+ tests):**

```
test_delete_parameter
test_delete_nonexistent_parameter
test_delete_parameters
test_get_parameters_by_path
test_put_parameter
test_put_parameter_overwrite_preserves_metadata
test_put_parameter_with_invalid_policy
test_put_parameter_empty_string_value
test_put_parameter_invalid_names
test_put_parameter_china
test_put_parameter_invalid_data_type
test_put_parameter_invalid_type
test_put_parameter_no_type
test_update_parameter
test_update_parameter_already_exists_error
test_get_parameter
test_get_parameter_with_version_and_labels
test_get_parameters_errors
test_get_nonexistant_parameter
test_describe_parameters
test_describe_parameters_paging
test_describe_parameters_filter_names
test_describe_parameters_filter_type
test_describe_parameters_filter_keyid
test_describe_parameters_with_parameter_filters_keyid
test_describe_parameters_with_parameter_filters_name
test_describe_parameters_with_parameter_filters_path
test_describe_parameters_needs_param
test_describe_parameters_invalid_parameter_filters
test_describe_parameters_invalid_path
test_describe_parameters_attributes
test_describe_parameters_tags
test_describe_parameters__multiple_tags
test_tags_in_list_tags_from_resource_parameter
test_tags_invalid_resource_id
test_tags_invalid_resource_type
test_get_parameter_invalid
test_put_parameter_secure_default_kms
test_put_parameter_secure_custom_kms
test_get_parameter_history
test_get_parameter_history_with_secure_string
test_label_parameter_version
test_label_parameter_version_with_specific_version
test_label_parameter_version_twice
test_label_parameter_moving_versions
test_label_parameter_moving_versions_complex
test_label_parameter_version_exception_ten_labels_at_once
test_label_parameter_version_exception_ten_labels_over_multiple_calls
test_label_parameter_version_invalid_name
test_label_parameter_version_invalid_parameter_version
test_label_parameter_version_invalid_label
test_get_parameter_history_with_label
test_get_parameter_history_with_label_non_latest
test_get_parameter_history_with_label_latest_assumed
test_get_parameter_history_missing_parameter
test_add_remove_list_tags_for_resource
test_send_command
test_list_commands
test_get_command_invocation
test_get_command_invocations_by_instance_tag
test_parameter_version_limit
test_parameter_overwrite_fails_when_limit_reached_and_oldest_version_has_label
test_get_parameters_includes_invalid_parameter_when_requesting_invalid_version
test_get_parameters_includes_invalid_parameter_when_requesting_invalid_label
test_get_parameters_should_only_return_unique_requests
test_get_parameter_history_should_throw_exception_when_MaxResults_is_too_large
```

**Why moto is valuable:** It is the most comprehensive mock of SSM Parameter Store, with 66+ tests covering edge cases like version limits, label management, filter validation, tag operations, and error conditions. The moto implementation in `moto/ssm/models.py` is an excellent reference implementation.

### 8.2 LocalStack

**Repository:** [localstack/localstack](https://github.com/localstack/localstack)
**Test file:** `tests/aws/services/ssm/test_ssm.py`

**Test functions (11 tests):**

```
test_describe_parameters
test_put_parameters
test_hierarchical_parameter
test_get_secret_parameter
test_get_inexistent_secret
test_get_parameters_and_secrets
test_get_parameters_by_path_and_filter_by_labels
test_get_parameter_by_arn
test_get_inexistent_maintenance_window
test_trigger_event_on_systems_manager_change
test_parameters_with_path
```

LocalStack's test suite is smaller but covers the core happy paths. It also tests EventBridge integration for parameter change events.

### 8.3 AWS SDK Integration Tests

The AWS SDK for Rust (`aws-sdk-ssm` crate on crates.io) includes examples but not a standalone compatibility test suite. The AWS SDK code examples for Rust demonstrate `GetParameter`, `PutParameter`, and `DescribeParameters`.

### 8.4 No Standalone SSM Compatibility Test Suite

Unlike S3 (which has MinIO Mint, Ceph s3-tests, etc.), there is **no standalone SSM compatibility test suite** analogous to those projects. The best testing strategy is to:

1. Port/adapt moto's Parameter Store tests (most comprehensive)
2. Write AWS SDK-based integration tests using the `aws-sdk-ssm` Rust crate
3. Test with Chamber as an end-to-end validation tool

---

## 9. Tools That Use SSM Parameter Store

### 9.1 Chamber (Segment.io)

**Repository:** [segmentio/chamber](https://github.com/segmentio/chamber)
**Language:** Go

Chamber is the most popular SSM Parameter Store management tool. Its SSM operations (from `store/ssmstore.go`):

| Chamber Operation | SSM API Calls |
|-------------------|---------------|
| `write` (create/update secret) | `PutParameter`, `AddTagsToResource` |
| `WriteTags` | `AddTagsToResource` |
| `readLatest` | `GetParameters`, `DescribeParameters` |
| `readVersion` | `GetParameterHistory` |
| `ListServices` | `DescribeParameters` |
| `List` (enumerate secrets) | `DescribeParameters`, `GetParameters` (batch of 10) |
| `ListRaw` | `GetParametersByPath` (with label filtering) |
| `History` | `GetParameterHistory` |
| `Delete` | `DeleteParameter` |
| `DeleteTags` | `RemoveTagsFromResource` |
| `ReadTags` | `ListTagsForResource` |

**Chamber uses ALL 10 core Parameter Store operations**, making it an excellent end-to-end validation tool.

Chamber expects a KMS key with alias `parameter_store_key` for encrypting SecureString parameters.

### 9.2 Spring Cloud AWS

Uses `GetParametersByPath` to load configuration from SSM into Spring Boot applications. Path-based hierarchy is the primary access pattern.

### 9.3 AWS CDK

Uses `GetParameter` (via `valueForStringParameter`) during synthesis. Also uses `PutParameter` and `AddTagsToResource` for resource creation.

### 9.4 Terraform AWS Provider

Uses `PutParameter`, `GetParameter`, `AddTagsToResource`, `RemoveTagsFromResource`, `ListTagsForResource`, `DeleteParameter` for SSM parameter resource management.

### 9.5 ssm-env (Rust)

A Rust utility (`dylanwh/ssm-env`) that loads environment variables from SSM. Uses `GetParametersByPath` primarily.

### 9.6 confd

Configuration management tool that uses SSM as a backend. Uses `GetParametersByPath`.

### 9.7 Doppler

Secret management platform. Syncs with SSM using `PutParameter` and `GetParametersByPath`.

### 9.8 Summary of Operations Used by Tools

| Operation | Chamber | Spring | CDK | Terraform | ssm-env | confd | Doppler |
|-----------|---------|--------|-----|-----------|---------|-------|---------|
| PutParameter | x | | x | x | | | x |
| GetParameter | | | x | x | | | |
| GetParameters | x | | | | | | |
| GetParametersByPath | x | x | | | x | x | x |
| GetParameterHistory | x | | | | | | |
| DescribeParameters | x | | | | | | |
| DeleteParameter | x | | | x | | | |
| DeleteParameters | | | | | | | |
| LabelParameterVersion | | | | | | | |
| UnlabelParameterVersion | | | | | | | |
| AddTagsToResource | x | | x | x | | | |
| RemoveTagsFromResource | x | | | x | | | |
| ListTagsForResource | x | | | x | | | |

---

## 10. Implementation Priority Matrix

### 10.1 Tier 1 -- MVP (Must Have)

These 6 operations cover the core CRUD workflow that every tool requires:

| # | Operation | Rationale |
|---|-----------|-----------|
| 1 | `PutParameter` | Create/update parameters |
| 2 | `GetParameter` | Read a single parameter |
| 3 | `GetParameters` | Batch read (Chamber, SDK) |
| 4 | `GetParametersByPath` | Path-based listing (Chamber, Spring, ssm-env, confd, Doppler) |
| 5 | `DeleteParameter` | Delete a parameter |
| 6 | `DeleteParameters` | Batch delete |

With these 6 operations, the following tools would work:
- AWS CLI (`aws ssm put-parameter`, `get-parameter`, `get-parameters`, `get-parameters-by-path`, `delete-parameter`, `delete-parameters`)
- ssm-env
- confd
- Basic Terraform workflows
- Basic Spring Cloud AWS
- Basic CDK synthesis

### 10.2 Tier 2 -- Core Features (Should Have)

| # | Operation | Rationale |
|---|-----------|-----------|
| 7 | `DescribeParameters` | Parameter search/listing (Chamber, Terraform) |
| 8 | `GetParameterHistory` | Version history (Chamber) |
| 9 | `AddTagsToResource` | Tag management (Chamber, Terraform) |
| 10 | `RemoveTagsFromResource` | Tag management (Chamber, Terraform) |
| 11 | `ListTagsForResource` | Tag management (Chamber, Terraform) |

With Tier 1 + Tier 2 (11 operations), **Chamber would fully work**, along with Terraform's full SSM resource lifecycle.

### 10.3 Tier 3 -- Advanced Features (Nice to Have)

| # | Operation | Rationale |
|---|-----------|-----------|
| 12 | `LabelParameterVersion` | Version labeling |
| 13 | `UnlabelParameterVersion` | Version unlabeling |

These are used less frequently in local dev scenarios but needed for full Parameter Store compatibility.

### 10.4 Tier 4 -- Service Settings (Optional)

| # | Operation | Rationale |
|---|-----------|-----------|
| 14 | `GetServiceSetting` | Throughput/tier config |
| 15 | `UpdateServiceSetting` | Throughput/tier config |
| 16 | `ResetServiceSetting` | Throughput/tier config |

These are rarely needed for local dev.

### 10.5 Implementation Scope Summary

- **MVP (Tier 1):** 6 operations -- covers basic CRUD, path queries
- **Full Parameter Store (Tier 1+2+3):** 13 operations -- covers all Parameter Store functionality
- **With service settings (all tiers):** 16 operations

For comparison: DynamoDB MVP was ~10 operations, full was ~20+. SSM Parameter Store is significantly smaller in scope.

---

## 11. Architecture Considerations for RustStack

### 11.1 Protocol Reuse

Since SSM uses `awsJson1.1` and DynamoDB uses `awsJson1.0`, the protocol handling infrastructure is nearly identical:
- Same JSON serialization/deserialization
- Same `X-Amz-Target` dispatch mechanism
- Same SigV4 authentication
- Only the Content-Type version and target prefix differ

The existing DynamoDB HTTP layer can be generalized into a shared `awsJson` protocol handler.

### 11.2 Suggested Crate Structure

Following the existing pattern:
- `ruststack-ssm-model` -- Auto-generated Smithy types (from Parameter Store subset of SSM model)
- `ruststack-ssm-core` -- Parameter Store business logic
- `ruststack-ssm-http` -- HTTP routing, service layer

Or, since SSM Parameter Store is much simpler than DynamoDB, it could be a single crate:
- `ruststack-ssm` -- Combined model, core, and HTTP (like a simpler S3)

### 11.3 Storage Model

Parameter Store storage is straightforward compared to DynamoDB:

```
Account -> Region -> Parameters (HashMap<String, ParameterRecord>)

ParameterRecord:
  - name: String
  - current_version: u64
  - versions: BTreeMap<u64, ParameterVersion>  // max 100 entries
  - tags: HashMap<String, String>  // max 50 entries

ParameterVersion:
  - value: String
  - type_: ParameterType
  - description: Option<String>
  - key_id: Option<String>
  - allowed_pattern: Option<String>
  - data_type: String
  - tier: ParameterTier
  - labels: HashSet<String>  // max 10 per version
  - policies: Vec<ParameterPolicy>
  - last_modified_date: f64
  - last_modified_user: String
```

The path hierarchy (`/a/b/c`) is just string matching on the parameter name -- no need for a tree structure. A `HashMap<String, ParameterRecord>` with prefix matching for `GetParametersByPath` is sufficient.

### 11.4 SecureString Handling

For local dev, SecureString parameters can be stored as plaintext with a marker indicating they are SecureString type. Actual KMS encryption is unnecessary. When `WithDecryption: false`, the value can be returned as-is (or optionally base64-encoded to simulate un-decrypted behavior).

### 11.5 Gateway Integration

SSM should be registered as another service in the RustStack gateway alongside S3 and DynamoDB. Service dispatch can use the `X-Amz-Target` header prefix:
- `DynamoDB_20120810.*` -> DynamoDB service
- `AmazonSSM.*` -> SSM service

### 11.6 Estimated Effort

Given the small API surface (13 operations vs DynamoDB's 66):
- **Tier 1 MVP:** 2-3 days
- **Tier 1+2+3 Full:** 4-5 days
- **Integration tests:** 1-2 days

SSM Parameter Store is one of the simplest AWS services to implement and would add significant value for local dev/CI users.

---

## Sources

- [AWS Systems Manager API Reference - Operations](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_Operations.html)
- [AWS SSM GetParameter API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameter.html)
- [AWS SSM PutParameter API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_PutParameter.html)
- [AWS SSM GetParametersByPath API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParametersByPath.html)
- [AWS SSM DescribeParameters API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_DescribeParameters.html)
- [AWS SSM GetParameters API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameters.html)
- [AWS SSM GetParameterHistory API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_GetParameterHistory.html)
- [AWS SSM DeleteParameter API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_DeleteParameter.html)
- [AWS SSM DeleteParameters API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_DeleteParameters.html)
- [AWS SSM LabelParameterVersion API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_LabelParameterVersion.html)
- [AWS SSM UnlabelParameterVersion API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_UnlabelParameterVersion.html)
- [AWS SSM AddTagsToResource API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_AddTagsToResource.html)
- [AWS SSM RemoveTagsFromResource API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_RemoveTagsFromResource.html)
- [AWS SSM ListTagsForResource API](https://docs.aws.amazon.com/systems-manager/latest/APIReference/API_ListTagsForResource.html)
- [AWS JSON 1.1 Protocol - Smithy 2.0](https://smithy.io/2.0/aws/protocols/aws-json-1_1-protocol.html)
- [aws/api-models-aws GitHub Repository](https://github.com/aws/api-models-aws)
- [AWS Smithy API Models Announcement](https://aws.amazon.com/blogs/aws/introducing-aws-api-models-and-publicly-available-resources-for-aws-api-definitions/)
- [getmoto/moto SSM Tests](https://github.com/getmoto/moto/blob/master/tests/test_ssm/test_ssm.py)
- [localstack/localstack SSM Tests](https://github.com/localstack/localstack)
- [segmentio/chamber SSM Store](https://github.com/segmentio/chamber/blob/master/store/ssmstore.go)
- [AWS SSM Parameter Store User Guide](https://docs.aws.amazon.com/systems-manager/latest/userguide/systems-manager-parameter-store.html)
- [Parameter Hierarchies](https://docs.aws.amazon.com/systems-manager/latest/userguide/sysman-paramstore-hierarchies.html)
- [Parameter Versions](https://docs.aws.amazon.com/systems-manager/latest/userguide/sysman-paramstore-versions.html)
- [Parameter Labels](https://docs.aws.amazon.com/systems-manager/latest/userguide/sysman-paramstore-labels.html)
- [aws-sdk-ssm Rust Crate](https://crates.io/crates/aws-sdk-ssm)
