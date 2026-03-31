# Rustack API Gateway v2: HTTP APIs Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [rustack-lambda-design.md](./rustack-lambda-design.md), [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md)
**Scope:** Add API Gateway v2 (HTTP APIs) support to Rustack -- management API for APIs, routes, integrations, stages, deployments, authorizers, models, domain names, VPC links, and API mappings, plus an execution engine for routing HTTP requests through configured API Gateway resources. ~47 operations across 5 phases, using the same Smithy-based codegen and gateway routing patterns established by Lambda.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: restJson1](#5-protocol-design-restjson1)
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

This spec proposes adding API Gateway v2 (HTTP APIs) support to Rustack. Key design decisions:

- **Two-component architecture** -- the service consists of a **management API** (CRUD for APIs, routes, integrations, stages, deployments, authorizers, models, domain names, VPC links) and an **execution engine** (actually routing HTTP requests through configured API Gateway resources to backend integrations). These are fundamentally different subsystems sharing a common storage layer.
- **restJson1 protocol** -- API Gateway v2 uses `restJson1`, the same protocol as Lambda. Operations are dispatched by HTTP method + URL path (e.g., `POST /v2/apis` for `CreateApi`, `GET /v2/apis/{apiId}/routes` for `GetRoutes`). The existing restJson1 codegen infrastructure from Lambda is fully reused.
- **Execution engine via `/_aws/execute-api/{apiId}/{stage}/{proxy+}`** -- incoming API execution requests are routed through a dedicated path prefix, matching LocalStack's pattern. The execution engine matches requests against configured routes, resolves the target integration (Lambda proxy, HTTP proxy, or mock), and dispatches accordingly. Lambda proxy integration invokes functions via the existing `rustack-lambda-core` crate.
- **Cross-service dependency on Lambda** -- Lambda proxy integration (`AWS_PROXY` with a Lambda ARN) creates a compile-time dependency on `rustack-lambda-core`. This is gated behind a cargo feature to keep the dependency optional.
- **Gateway routing by `/v2/` URL prefix** -- all API Gateway v2 management API requests use URL paths starting with `/v2/apis`, `/v2/domainnames`, or `/v2/vpclinks`. The gateway routes these prefixes to the API Gateway v2 service before falling through to S3.
- **Phased delivery** -- 5 phases from core API management (APIs, routes, integrations) through the execution engine, with the execution engine as Phase 4 after the full management API is in place.

---

## 2. Motivation

### 2.1 Why API Gateway v2?

API Gateway is the front door for serverless applications on AWS. The combination of API Gateway + Lambda is the single most common serverless pattern, forming the foundation of virtually every serverless REST API, webhook handler, and microservice on AWS. Developers need a local API Gateway for:

- **End-to-end serverless testing** -- test the complete request flow from HTTP request through API Gateway routing to Lambda invocation and back. Without a local API Gateway, developers can only test Lambda functions in isolation, missing route matching, parameter mapping, CORS handling, and integration configuration bugs.
- **SAM CLI integration** -- `sam local start-api` relies on API Gateway configuration to route requests to local Lambda functions. SAM reads the CloudFormation/SAM template, extracts API Gateway routes, and needs a compatible API endpoint.
- **CDK local testing** -- AWS CDK's `HttpApi` and `HttpLambdaIntegration` constructs generate API Gateway v2 resources that need a local endpoint for integration testing.
- **Serverless Framework** -- the most widely used serverless deployment tool creates API Gateway v2 HTTP APIs by default (since v3). The `serverless-offline` plugin emulates this behavior but is Node.js-specific and not protocol-compatible.
- **Terraform** -- `aws_apigatewayv2_api`, `aws_apigatewayv2_route`, `aws_apigatewayv2_integration`, and `aws_apigatewayv2_stage` are among the most commonly used Terraform resources for serverless infrastructure.
- **CI/CD pipelines** -- fast integration testing in GitHub Actions without AWS credentials or network access.
- **Offline development** -- work without internet connectivity while testing full API flows.

### 2.2 API Gateway v1 vs v2

API Gateway v2 (HTTP APIs) is the modern, simplified version of API Gateway. It supersedes API Gateway v1 (REST APIs) for most use cases:

| Aspect | API Gateway v1 (REST) | API Gateway v2 (HTTP) |
|--------|----------------------|----------------------|
| Protocol | REST API | HTTP API |
| Cost | $3.50 per million requests | $1.00 per million requests |
| Latency | Higher | Lower (up to 50% faster) |
| JWT authorizers | Not native (custom Lambda) | Built-in |
| CORS | Manual configuration | Automatic |
| Default route | Not supported | Supported (`$default`) |
| Auto-deploy stages | Not supported | Built-in |
| SAM/CDK default | `Api` (legacy) | `HttpApi` (preferred) |
| Service name | `apigateway` (v1) | `apigatewayv2` (v2) |

API Gateway v2 is the recommended choice for new projects and is the default in modern tooling. Rustack implements v2 first because it covers the dominant use case and has a simpler API surface.

### 2.3 Complexity Assessment

| Dimension | API Gateway v2 | Lambda | Secrets Manager |
|-----------|---------------|--------|-----------------|
| Total operations | ~47 | 27 | 23 |
| Complex subsystems | 2 (management + execution) | 2 (management + Docker) | 1 (version staging) |
| Storage complexity | Nested hierarchy (API > Routes > Integrations) | Flat (functions + versions) | Flat (secrets + versions) |
| Cross-service deps | Lambda (for proxy integration) | Docker (external) | None |
| Protocol | restJson1 (reuse Lambda) | restJson1 | awsJson1.1 |
| Estimated lines of code | ~8,000-10,000 | ~12,500 | ~4,500 |

API Gateway v2 is moderately complex. The management API is straightforward CRUD, but the execution engine introduces significant complexity around route matching, integration dispatch, request/response transformation, and CORS handling.

### 2.4 Tool Coverage

With the target operations implemented, the following tools work:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws apigatewayv2`) | All CRUD ops | Phase 0 |
| SAM CLI (`sam local start-api`) | CreateApi, CreateRoute, CreateIntegration, CreateStage | Phase 0 + Phase 4 |
| Serverless Framework | CreateApi, CreateRoute, CreateIntegration, CreateStage, CreateDeployment | Phase 0 + Phase 1 |
| AWS CDK (`HttpApi`) | CreateApi, CreateRoute, CreateIntegration, CreateStage, auto-deploy | Phase 0 + Phase 1 |
| Terraform | Full CRUD on all resource types | All phases |
| boto3 / aws-sdk-rust | All operations | All phases |
| curl (API execution) | Execute configured HTTP APIs | Phase 4 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **HTTP API management** -- full CRUD for APIs, routes, integrations, stages, deployments, route responses, authorizers, models, domain names, VPC links, API mappings, and tags (~47 operations)
2. **HTTP API execution** -- route incoming HTTP requests through configured API Gateway resources to backend integrations (Lambda proxy, HTTP proxy, mock)
3. **Lambda proxy integration** -- `AWS_PROXY` integration type invokes Lambda functions via `rustack-lambda-core`, passing the request as an API Gateway v2 event payload and mapping the Lambda response back to HTTP
4. **HTTP proxy integration** -- `HTTP_PROXY` integration type forwards requests to backend HTTP URLs
5. **Mock integration** -- `MOCK` integration type returns configured static responses
6. **Route matching** -- path template matching with parameters (e.g., `/items/{id}` matches `/items/123`), greedy path parameters (`{proxy+}`), and the `$default` catch-all route
7. **CORS handling** -- API-level `cors_configuration` with automatic preflight response generation
8. **Auto-deploy stages** -- when `auto_deploy` is enabled on a stage, changes to routes/integrations automatically create new deployments
9. **Stage variables** -- key-value pairs available during request execution for integration configuration
10. **restJson1 protocol** -- URL-based routing with proper HTTP method/path/status code binding, reusing Lambda's restJson1 infrastructure
11. **Smithy-generated types** -- all API Gateway v2 types generated from official AWS Smithy model
12. **Same Docker image** -- single binary serves all services on port 4566
13. **JWT authorizers** -- validate JWT tokens against configured issuers and audiences (store configuration, basic validation)

### 3.2 Non-Goals

1. **WebSocket APIs** -- API Gateway v2 supports both HTTP and WebSocket APIs. WebSocket support (connection management, `$connect`/`$disconnect`/`$default` routes, two-way communication) is out of scope. Accept `protocol_type: WEBSOCKET` but return an error on execution.
2. **API Gateway v1 (REST APIs)** -- the legacy REST API service (`apigateway` v1) is a separate service with different operations, resource hierarchy (RestApi > Resource > Method), and execution semantics. Out of scope entirely.
3. **Custom domain HTTPS** -- accept domain name configuration but do not provision TLS certificates or DNS records.
4. **VPC link actual connectivity** -- accept VPC link configuration but do not create actual VPC network connections.
5. **AWS IAM authorization** -- accept `AWS_IAM` authorization type but do not evaluate IAM policies. Requests pass through regardless.
6. **Request/response parameter mapping** -- API Gateway v2 supports request/response parameter mapping expressions. Accept and store configuration but do not evaluate mapping expressions in the execution engine for the initial implementation.
7. **Access logging** -- accept access log configuration but do not emit logs to CloudWatch Logs.
8. **Throttling** -- accept throttling configuration (route-level and stage-level) but do not enforce rate limits.
9. **Mutual TLS** -- accept mTLS configuration but do not enforce client certificate validation.
10. **Private integrations** -- accept private integration configuration but do not route through VPC links.
11. **Data persistence across restarts** -- in-memory only, matching other services.
12. **Lambda authorizers** -- `REQUEST` type authorizers that invoke a Lambda function for authorization. Store configuration but do not invoke the authorizer Lambda. JWT authorizers with basic validation are in scope.

---

## 4. Architecture Overview

### 4.1 Two-Component Architecture

API Gateway v2 has two fundamentally different responsibilities:

```
                    AWS SDK / CLI / SAM CLI / Terraform
                         |
                         | HTTP :4566
                         v
              +---------------------+
              |   Gateway Router    |  Routes by URL prefix
              |   (ServiceRouter)   |
              +--------+------------+
                       |
         +-------------+------- ... -----+
         |             |                 |
    +--------+   +-----------+    +-----------+
    |  APIGW | | Lambda   |    | S3        |
    |  v2    | | RestJ1   |    | (catch-all)|
    | RestJ1 | |          |    |           |
    +---+----+ +----------+    +-----------+
        |
        +------------------+
        |                  |
   +---------+      +-----------+
   |Management|     | Execution |
   |  API     |     |  Engine   |
   | (CRUD)   |     | (Proxy)   |
   +---------+      +-----+-----+
        |                  |
   +---------+      +------+------+
   | Storage |      | Lambda Core |  (cross-service)
   | (DashMap)|     | HTTP Client |
   +---------+      +-------------+
```

**Management API**: Handles all CRUD operations for API Gateway v2 resources. This is a standard restJson1 service with path-based routing (`/v2/apis`, `/v2/apis/{apiId}/routes`, etc.). Registered in the gateway to handle `/v2/` path prefix.

**Execution Engine**: Handles actual API execution -- routing incoming HTTP requests through configured API Gateway routes to backend integrations. Registered in the gateway to handle `/_aws/execute-api/{apiId}/{stage}/{proxy+}` path prefix.

### 4.2 Gateway Routing

API Gateway v2 requires TWO routing registrations in the gateway:

| Signal | Management API | Execution Engine |
|--------|---------------|------------------|
| URL Path | `/v2/apis*`, `/v2/domainnames*`, `/v2/vpclinks*`, `/v2/tags*` | `/_aws/execute-api/{apiId}/{stage}/*` |
| HTTP Method | GET, POST, PUT, PATCH, DELETE | Any (depends on configured routes) |
| Content-Type | `application/json` | Any (depends on integration) |
| Purpose | CRUD for API GW resources | Proxy requests to backends |

**Routing logic** (evaluated in order, additions shown with `<--`):

1. If URL path starts with `/v2/apis`, `/v2/domainnames`, `/v2/vpclinks`, or `/v2/tags` -- route to API Gateway v2 management API `<-- NEW`
2. If URL path starts with `/_aws/execute-api/` -- route to API Gateway v2 execution engine `<-- NEW`
3. If URL path starts with `/2015-03-31/functions` or `/2021-10-31/functions` -- route to Lambda
4. If `X-Amz-Target` starts with `DynamoDB_` -- route to DynamoDB
5. If `X-Amz-Target` starts with `AmazonSQS` -- route to SQS
6. If `X-Amz-Target` starts with `AmazonSSM.` -- route to SSM
7. If `X-Amz-Target` starts with `secretsmanager.` -- route to Secrets Manager
8. *(other header-based services)*
9. Default: route to S3 (catch-all)

The `/v2/` prefix is unambiguous and cannot conflict with S3 bucket names or other services. The `/_aws/execute-api/` prefix follows LocalStack's convention for API execution endpoints.

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-apigatewayv2-model      <-- NEW (auto-generated)
+-- rustack-apigatewayv2-http       <-- NEW
+-- rustack-apigatewayv2-core       <-- NEW
+-- rustack-lambda-{model,core,http}
+-- rustack-s3-{model,core,http}
+-- ... (other services)

rustack-apigatewayv2-http
+-- rustack-apigatewayv2-model
+-- rustack-auth

rustack-apigatewayv2-core
+-- rustack-core
+-- rustack-apigatewayv2-model
+-- rustack-lambda-core (optional, for Lambda proxy integration)
+-- reqwest (for HTTP proxy integration)
+-- dashmap
+-- tokio
```

The dependency on `rustack-lambda-core` is gated behind a `lambda-integration` feature flag. When disabled, Lambda proxy integrations return an error indicating that Lambda support is not available.

---

## 5. Protocol Design: restJson1

### 5.1 Protocol Characteristics

API Gateway v2 uses the `restJson1` Smithy protocol, identical to Lambda. The existing restJson1 infrastructure (router, path parameter extraction, request deserialization, response serialization, error formatting) is fully reused.

| Aspect | awsJson (DDB/SQS/SSM) | restJson1 (Lambda/APIGW v2) |
|--------|----------------------|------------------------------|
| HTTP Method | POST only | GET, POST, PUT, PATCH, DELETE |
| URL Path | Always `/` | Operation-specific (e.g., `/v2/apis/{apiId}`) |
| Operation dispatch | `X-Amz-Target` header | HTTP method + URL path matching |
| Request params | All in JSON body | Split across path, query, headers, and body |
| Response status | Always 200 (success) | Operation-specific (200, 201, 204) |
| Error type header | `__type` in body | `X-Amzn-Errortype` |
| Content-Type | `application/x-amz-json-*` | `application/json` |
| JSON field casing | PascalCase (DDB) or camelCase | camelCase |

### 5.2 Request Anatomy

A typical API Gateway v2 request:

```http
POST /v2/apis HTTP/1.1
Content-Type: application/json
Authorization: AWS4-HMAC-SHA256 ...

{
  "name": "my-http-api",
  "protocolType": "HTTP",
  "corsConfiguration": {
    "allowOrigins": ["*"],
    "allowMethods": ["GET", "POST"],
    "allowHeaders": ["content-type"]
  }
}
```

A route creation request with path parameters:

```http
POST /v2/apis/abc123/routes HTTP/1.1
Content-Type: application/json

{
  "routeKey": "GET /items/{id}",
  "target": "integrations/def456",
  "authorizationType": "NONE"
}
```

### 5.3 Response Anatomy

```http
HTTP/1.1 201 Created
Content-Type: application/json

{
  "apiId": "abc123",
  "name": "my-http-api",
  "protocolType": "HTTP",
  "routeSelectionExpression": "${request.method} ${request.path}",
  "corsConfiguration": {
    "allowOrigins": ["*"],
    "allowMethods": ["GET", "POST"],
    "allowHeaders": ["content-type"]
  },
  "createdDate": "2026-03-19T10:30:00Z",
  "apiEndpoint": "http://localhost:4566/_aws/execute-api/abc123"
}
```

### 5.4 Error Response Format

```http
HTTP/1.1 404 Not Found
Content-Type: application/json
X-Amzn-Errortype: NotFoundException

{
  "message": "Invalid API identifier specified abc123"
}
```

Note: API Gateway v2 uses lowercase `message` in error responses (unlike Lambda which uses `Message`).

### 5.5 API Gateway v2 Management API Route Table

All operations with their HTTP bindings:

| Operation | Method | Path | Success Status | Phase |
|-----------|--------|------|---------------|-------|
| **CreateApi** | POST | `/v2/apis` | 201 | 0 |
| **GetApi** | GET | `/v2/apis/{apiId}` | 200 | 0 |
| **UpdateApi** | PATCH | `/v2/apis/{apiId}` | 200 | 0 |
| **DeleteApi** | DELETE | `/v2/apis/{apiId}` | 204 | 0 |
| **GetApis** | GET | `/v2/apis` | 200 | 0 |
| **CreateRoute** | POST | `/v2/apis/{apiId}/routes` | 201 | 0 |
| **GetRoute** | GET | `/v2/apis/{apiId}/routes/{routeId}` | 200 | 0 |
| **UpdateRoute** | PATCH | `/v2/apis/{apiId}/routes/{routeId}` | 200 | 0 |
| **DeleteRoute** | DELETE | `/v2/apis/{apiId}/routes/{routeId}` | 204 | 0 |
| **GetRoutes** | GET | `/v2/apis/{apiId}/routes` | 200 | 0 |
| **CreateIntegration** | POST | `/v2/apis/{apiId}/integrations` | 201 | 0 |
| **GetIntegration** | GET | `/v2/apis/{apiId}/integrations/{integrationId}` | 200 | 0 |
| **UpdateIntegration** | PATCH | `/v2/apis/{apiId}/integrations/{integrationId}` | 200 | 0 |
| **DeleteIntegration** | DELETE | `/v2/apis/{apiId}/integrations/{integrationId}` | 204 | 0 |
| **GetIntegrations** | GET | `/v2/apis/{apiId}/integrations` | 200 | 0 |
| **CreateStage** | POST | `/v2/apis/{apiId}/stages` | 201 | 1 |
| **GetStage** | GET | `/v2/apis/{apiId}/stages/{stageName}` | 200 | 1 |
| **UpdateStage** | PATCH | `/v2/apis/{apiId}/stages/{stageName}` | 200 | 1 |
| **DeleteStage** | DELETE | `/v2/apis/{apiId}/stages/{stageName}` | 204 | 1 |
| **GetStages** | GET | `/v2/apis/{apiId}/stages` | 200 | 1 |
| **CreateDeployment** | POST | `/v2/apis/{apiId}/deployments` | 201 | 1 |
| **GetDeployment** | GET | `/v2/apis/{apiId}/deployments/{deploymentId}` | 200 | 1 |
| **DeleteDeployment** | DELETE | `/v2/apis/{apiId}/deployments/{deploymentId}` | 204 | 1 |
| **GetDeployments** | GET | `/v2/apis/{apiId}/deployments` | 200 | 1 |
| **CreateRouteResponse** | POST | `/v2/apis/{apiId}/routes/{routeId}/routeresponses` | 201 | 1 |
| **GetRouteResponse** | GET | `/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}` | 200 | 1 |
| **DeleteRouteResponse** | DELETE | `/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}` | 204 | 1 |
| **GetRouteResponses** | GET | `/v2/apis/{apiId}/routes/{routeId}/routeresponses` | 200 | 1 |
| **CreateAuthorizer** | POST | `/v2/apis/{apiId}/authorizers` | 201 | 2 |
| **GetAuthorizer** | GET | `/v2/apis/{apiId}/authorizers/{authorizerId}` | 200 | 2 |
| **UpdateAuthorizer** | PATCH | `/v2/apis/{apiId}/authorizers/{authorizerId}` | 200 | 2 |
| **DeleteAuthorizer** | DELETE | `/v2/apis/{apiId}/authorizers/{authorizerId}` | 204 | 2 |
| **GetAuthorizers** | GET | `/v2/apis/{apiId}/authorizers` | 200 | 2 |
| **CreateModel** | POST | `/v2/apis/{apiId}/models` | 201 | 2 |
| **GetModel** | GET | `/v2/apis/{apiId}/models/{modelId}` | 200 | 2 |
| **UpdateModel** | PATCH | `/v2/apis/{apiId}/models/{modelId}` | 200 | 2 |
| **DeleteModel** | DELETE | `/v2/apis/{apiId}/models/{modelId}` | 204 | 2 |
| **GetModels** | GET | `/v2/apis/{apiId}/models` | 200 | 2 |
| **GetModelTemplate** | GET | `/v2/apis/{apiId}/models/{modelId}/template` | 200 | 2 |
| **CreateDomainName** | POST | `/v2/domainnames` | 201 | 3 |
| **GetDomainName** | GET | `/v2/domainnames/{domainName}` | 200 | 3 |
| **DeleteDomainName** | DELETE | `/v2/domainnames/{domainName}` | 204 | 3 |
| **GetDomainNames** | GET | `/v2/domainnames` | 200 | 3 |
| **CreateVpcLink** | POST | `/v2/vpclinks` | 201 | 3 |
| **GetVpcLink** | GET | `/v2/vpclinks/{vpcLinkId}` | 200 | 3 |
| **DeleteVpcLink** | DELETE | `/v2/vpclinks/{vpcLinkId}` | 202 | 3 |
| **GetVpcLinks** | GET | `/v2/vpclinks` | 200 | 3 |
| **TagResource** | POST | `/v2/tags/{resource-arn}` | 201 | 3 |
| **UntagResource** | DELETE | `/v2/tags/{resource-arn}` | 204 | 3 |
| **GetTags** | GET | `/v2/tags/{resource-arn}` | 200 | 3 |
| **CreateApiMapping** | POST | `/v2/domainnames/{domainName}/apimappings` | 201 | 3 |
| **GetApiMapping** | GET | `/v2/domainnames/{domainName}/apimappings/{apiMappingId}` | 200 | 3 |
| **DeleteApiMapping** | DELETE | `/v2/domainnames/{domainName}/apimappings/{apiMappingId}` | 204 | 3 |
| **GetApiMappings** | GET | `/v2/domainnames/{domainName}/apimappings` | 200 | 3 |
| **UpdateDomainName** | PATCH | `/v2/domainnames/{domainName}` | 200 | 3 |
| **UpdateVpcLink** | PATCH | `/v2/vpclinks/{vpcLinkId}` | 200 | 3 |
| **UpdateApiMapping** | PATCH | `/v2/domainnames/{domainName}/apimappings/{apiMappingId}` | 200 | 3 |

Total: **~53 operations** across APIs, routes, integrations, stages, deployments, route responses, authorizers, models, domain names, VPC links, API mappings, and tags.

---

## 6. Smithy Code Generation Strategy

### 6.1 Approach: Reuse restJson1 Codegen from Lambda

The existing codegen supports `restJson1` for Lambda. API Gateway v2 uses the same protocol, so the codegen infrastructure (route metadata generation, field binding annotations, operation router generation) is directly reusable. The key differences are:

- API Gateway v2 uses `/v2/` path prefix (vs Lambda's `/2015-03-31/`)
- API Gateway v2 uses `PATCH` method for updates (vs Lambda's `PUT`)
- API Gateway v2 uses camelCase JSON fields (same as Lambda)
- API Gateway v2 has deeper nested paths (e.g., `/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}`)

### 6.2 Codegen Service Config (TOML)

```toml
# codegen/services/apigatewayv2.toml

[service]
name = "apigatewayv2"
display_name = "ApiGatewayV2"
rust_prefix = "ApiGatewayV2"
namespace = "com.amazonaws.apigatewayv2"
protocol = "restJson1"

[protocol]
serde_rename = "camelCase"
emit_serde_derives = true
emit_http_bindings = true

[operations]
phase0 = [
    # API CRUD
    "CreateApi", "GetApi", "UpdateApi", "DeleteApi", "GetApis",
    # Route CRUD
    "CreateRoute", "GetRoute", "UpdateRoute", "DeleteRoute", "GetRoutes",
    # Integration CRUD
    "CreateIntegration", "GetIntegration", "UpdateIntegration",
    "DeleteIntegration", "GetIntegrations",
]
phase1 = [
    # Stages + Deployments + Route Responses
    "CreateStage", "GetStage", "UpdateStage", "DeleteStage", "GetStages",
    "CreateDeployment", "GetDeployment", "DeleteDeployment", "GetDeployments",
    "CreateRouteResponse", "GetRouteResponse", "DeleteRouteResponse",
    "GetRouteResponses",
]
phase2 = [
    # Authorizers + Models
    "CreateAuthorizer", "GetAuthorizer", "UpdateAuthorizer",
    "DeleteAuthorizer", "GetAuthorizers",
    "CreateModel", "GetModel", "UpdateModel", "DeleteModel",
    "GetModels", "GetModelTemplate",
]
phase3 = [
    # Domain Names + VPC Links + Tags + API Mappings
    "CreateDomainName", "GetDomainName", "UpdateDomainName",
    "DeleteDomainName", "GetDomainNames",
    "CreateVpcLink", "GetVpcLink", "UpdateVpcLink",
    "DeleteVpcLink", "GetVpcLinks",
    "TagResource", "UntagResource", "GetTags",
    "CreateApiMapping", "GetApiMapping", "UpdateApiMapping",
    "DeleteApiMapping", "GetApiMappings",
]

[errors.custom]
UnknownOperation = { status = 500, message = "Unknown operation" }

[output]
file_layout = "flat"
```

### 6.3 Generated Route Metadata

For each operation, the codegen emits a route descriptor:

```rust
/// Auto-generated route table for API Gateway v2 operations.
pub struct ApiGatewayV2Route {
    pub method: http::Method,
    pub path_pattern: &'static str,
    pub operation: ApiGatewayV2Operation,
    pub success_status: u16,
}

pub const APIGATEWAYV2_ROUTES: &[ApiGatewayV2Route] = &[
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis",
        operation: ApiGatewayV2Operation::CreateApi,
        success_status: 201,
    },
    ApiGatewayV2Route {
        method: http::Method::GET,
        path_pattern: "/v2/apis/{apiId}",
        operation: ApiGatewayV2Operation::GetApi,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::PATCH,
        path_pattern: "/v2/apis/{apiId}",
        operation: ApiGatewayV2Operation::UpdateApi,
        success_status: 200,
    },
    ApiGatewayV2Route {
        method: http::Method::POST,
        path_pattern: "/v2/apis/{apiId}/routes",
        operation: ApiGatewayV2Operation::CreateRoute,
        success_status: 201,
    },
    // ... all ~53 operations
];
```

### 6.4 Generated Types Estimate

From the ~53 operations, the codegen produces roughly:
- 53 input structs (e.g., `CreateApiInput`, `CreateRouteInput`)
- 53 output structs (e.g., `CreateApiOutput`, `GetRouteOutput`)
- ~30 shared types (`Api`, `Route`, `Integration`, `Stage`, `Deployment`, `Authorizer`, `Model`, `DomainName`, `VpcLink`, `ApiMapping`, `Cors`, `IntegrationType`, `AuthorizationType`, `ProtocolType`, `ConnectionType`, `ContentHandlingStrategy`, `PassthroughBehavior`, etc.)
- 1 operation enum (`ApiGatewayV2Operation` with ~53 variants)
- ~53 route descriptors
- ~10 error types

Total: roughly 4,000-5,000 lines of generated code.

### 6.5 Smithy Model Acquisition

The API Gateway v2 Smithy model is available from:
- **Repository:** `https://github.com/aws/aws-models`
- **Path:** `apigatewayv2/smithy/model.json`
- Download and place at `codegen/smithy-model/apigatewayv2.json`

### 6.6 Makefile Integration

```makefile
codegen-apigatewayv2:
	@cd codegen && cargo run -- --service apigatewayv2
	@cargo +nightly fmt -p rustack-apigatewayv2-model

codegen: codegen-s3 codegen-dynamodb codegen-sqs codegen-ssm codegen-lambda codegen-apigatewayv2
```

---

## 7. Crate Structure

### 7.1 New Crates

#### `rustack-apigatewayv2-model` (auto-generated)

```
crates/rustack-apigatewayv2-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs                    # Module re-exports
    +-- types.rs                  # ProtocolType, IntegrationType, AuthorizationType, Cors, etc.
    +-- operations.rs             # ApiGatewayV2Operation enum + route table
    +-- error.rs                  # ApiGatewayV2Error + error codes
    +-- input/
    |   +-- mod.rs
    |   +-- api.rs                # CreateApiInput, GetApiInput, UpdateApiInput, etc.
    |   +-- route.rs              # CreateRouteInput, GetRouteInput, etc.
    |   +-- integration.rs        # CreateIntegrationInput, etc.
    |   +-- stage.rs              # CreateStageInput, etc.
    |   +-- deployment.rs         # CreateDeploymentInput, etc.
    |   +-- route_response.rs     # CreateRouteResponseInput, etc.
    |   +-- authorizer.rs         # CreateAuthorizerInput, etc.
    |   +-- model.rs              # CreateModelInput, etc.
    |   +-- domain_name.rs        # CreateDomainNameInput, etc.
    |   +-- vpc_link.rs           # CreateVpcLinkInput, etc.
    |   +-- tag.rs                # TagResourceInput, UntagResourceInput, GetTagsInput
    |   +-- api_mapping.rs        # CreateApiMappingInput, etc.
    +-- output/
        +-- mod.rs
        +-- api.rs
        +-- route.rs
        +-- integration.rs
        +-- stage.rs
        +-- deployment.rs
        +-- route_response.rs
        +-- authorizer.rs
        +-- model.rs
        +-- domain_name.rs
        +-- vpc_link.rs
        +-- tag.rs
        +-- api_mapping.rs
```

**Dependencies**: `serde`, `serde_json`, `http`

#### `rustack-apigatewayv2-http`

```
crates/rustack-apigatewayv2-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs                 # URL path + method pattern matching -> ApiGatewayV2Operation
    +-- dispatch.rs               # ApiGatewayV2Handler trait + dispatch logic
    +-- service.rs                # Hyper Service impl for API Gateway v2
    +-- request.rs                # restJson1 request deserialization
    +-- response.rs               # restJson1 response serialization
    +-- error.rs                  # Error response with X-Amzn-Errortype header
    +-- body.rs                   # Response body type
```

**Dependencies**: `rustack-apigatewayv2-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`, `http`

#### `rustack-apigatewayv2-core`

```
crates/rustack-apigatewayv2-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs                 # ApiGatewayV2Config
    +-- provider.rs               # RustackApiGatewayV2 (main provider)
    +-- handler.rs                # Handler bridging HTTP to provider
    +-- error.rs                  # ApiGatewayV2ServiceError
    +-- storage.rs                # All DashMap-based storage (ApiStore)
    +-- execution/
    |   +-- mod.rs
    |   +-- engine.rs             # ExecutionEngine: route matching + integration dispatch
    |   +-- router.rs             # Dynamic route matcher (path templates, $default)
    |   +-- lambda_proxy.rs       # Lambda proxy integration handler
    |   +-- http_proxy.rs         # HTTP proxy integration handler
    |   +-- mock.rs               # Mock integration handler
    |   +-- cors.rs               # CORS preflight and response header handling
    |   +-- event.rs              # API Gateway v2 event payload construction
    +-- ops/
        +-- mod.rs
        +-- api.rs                # CreateApi, GetApi, UpdateApi, DeleteApi, GetApis
        +-- route.rs              # CreateRoute, GetRoute, UpdateRoute, DeleteRoute, GetRoutes
        +-- integration.rs        # CreateIntegration, GetIntegration, UpdateIntegration, etc.
        +-- stage.rs              # CreateStage, GetStage, UpdateStage, DeleteStage, GetStages
        +-- deployment.rs         # CreateDeployment, GetDeployment, DeleteDeployment, etc.
        +-- route_response.rs     # Route response CRUD
        +-- authorizer.rs         # Authorizer CRUD
        +-- model.rs              # Model CRUD
        +-- domain_name.rs        # Domain name CRUD
        +-- vpc_link.rs           # VPC link CRUD
        +-- tag.rs                # TagResource, UntagResource, GetTags
        +-- api_mapping.rs        # API mapping CRUD
```

**Dependencies**: `rustack-core`, `rustack-apigatewayv2-model`, `rustack-lambda-core` (optional), `reqwest` (for HTTP proxy), `dashmap`, `tokio`, `uuid`, `chrono`, `tracing`, `regex`

### 7.2 Workspace Changes

```toml
[workspace.dependencies]
# ... existing deps ...
rustack-apigatewayv2-model = { path = "crates/rustack-apigatewayv2-model" }
rustack-apigatewayv2-http = { path = "crates/rustack-apigatewayv2-http" }
rustack-apigatewayv2-core = { path = "crates/rustack-apigatewayv2-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Management API Router

The management API router reuses the restJson1 pattern from Lambda. It matches incoming requests against the generated route table:

```rust
/// API Gateway v2 operation router for the management API.
///
/// Matches incoming HTTP requests against the API Gateway v2 route table
/// using method + path pattern matching. Paths are under the /v2/ prefix.
pub struct ApiGatewayV2Router;

impl ApiGatewayV2Router {
    /// Resolve an HTTP request to an API Gateway v2 operation.
    ///
    /// Returns the matched operation and extracted path parameters.
    pub fn resolve(
        method: &http::Method,
        path: &str,
    ) -> Result<(ApiGatewayV2Operation, PathParams), ApiGatewayV2Error> {
        // Routes ordered by specificity (most specific first):
        //   /v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}
        //   /v2/apis/{apiId}/routes/{routeId}/routeresponses
        //   /v2/apis/{apiId}/routes/{routeId}
        //   /v2/apis/{apiId}/routes
        //   /v2/apis/{apiId}/integrations/{integrationId}
        //   /v2/apis/{apiId}/integrations
        //   /v2/apis/{apiId}/authorizers/{authorizerId}
        //   /v2/apis/{apiId}/authorizers
        //   /v2/apis/{apiId}/models/{modelId}/template
        //   /v2/apis/{apiId}/models/{modelId}
        //   /v2/apis/{apiId}/models
        //   /v2/apis/{apiId}/stages/{stageName}
        //   /v2/apis/{apiId}/stages
        //   /v2/apis/{apiId}/deployments/{deploymentId}
        //   /v2/apis/{apiId}/deployments
        //   /v2/apis/{apiId}
        //   /v2/apis
        //   /v2/domainnames/{domainName}/apimappings/{apiMappingId}
        //   /v2/domainnames/{domainName}/apimappings
        //   /v2/domainnames/{domainName}
        //   /v2/domainnames
        //   /v2/vpclinks/{vpcLinkId}
        //   /v2/vpclinks
        //   /v2/tags/{resource-arn}

        for route in APIGATEWAYV2_ROUTES {
            if *method == route.method {
                if let Some(params) = match_path(path, route.path_pattern) {
                    return Ok((route.operation, params));
                }
            }
        }

        Err(ApiGatewayV2Error::unknown_operation(method, path))
    }
}
```

### 8.2 Execution Engine Router

The execution engine has a separate HTTP handler that intercepts requests matching `/_aws/execute-api/{apiId}/{stage}/{proxy+}`:

```rust
/// Execution engine router for API Gateway v2.
///
/// Intercepts requests to /_aws/execute-api/{apiId}/{stage}/{path...}
/// and routes them through configured API Gateway resources.
pub struct ExecutionRouter;

impl ExecutionRouter {
    /// Parse an execution request URL into its components.
    ///
    /// Input: /_aws/execute-api/abc123/prod/items/42
    /// Output: (api_id="abc123", stage="prod", path="/items/42")
    pub fn parse_execution_path(
        path: &str,
    ) -> Option<ExecutionTarget> {
        let stripped = path.strip_prefix("/_aws/execute-api/")?;
        let (api_id, rest) = stripped.split_once('/')?;
        let (stage, proxy_path) = rest.split_once('/')
            .map(|(s, p)| (s, format!("/{p}")))
            .unwrap_or((rest, "/".to_string()));

        Some(ExecutionTarget {
            api_id: api_id.to_string(),
            stage: stage.to_string(),
            path: proxy_path,
        })
    }
}

/// Parsed execution target from the URL.
#[derive(Debug, Clone)]
pub struct ExecutionTarget {
    /// The API ID being invoked.
    pub api_id: String,
    /// The stage name.
    pub stage: String,
    /// The proxy path (e.g., "/items/42").
    pub path: String,
}
```

### 8.3 ApiGatewayV2Handler Trait

```rust
/// The boundary between HTTP and business logic for the management API.
pub trait ApiGatewayV2Handler: Send + Sync + 'static {
    fn handle_operation(
        &self,
        op: ApiGatewayV2Operation,
        path_params: PathParams,
        query: String,
        headers: http::HeaderMap,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<ApiGatewayV2ResponseBody>, ApiGatewayV2Error>> + Send>>;
}
```

### 8.4 Execution Handler Trait

```rust
/// The execution engine handler for proxying API requests.
pub trait ExecutionHandler: Send + Sync + 'static {
    fn handle_execution(
        &self,
        target: ExecutionTarget,
        method: http::Method,
        headers: http::HeaderMap,
        query: String,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<Bytes>, ApiGatewayV2Error>> + Send>>;
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The storage engine manages the full API Gateway v2 resource hierarchy. All resources are nested under an API: an API contains routes, integrations, stages, deployments, authorizers, and models. Domain names and VPC links are top-level resources.

### 9.2 Core Data Structures

```rust
/// Top-level store for API Gateway v2 resources.
#[derive(Debug)]
pub struct ApiStore {
    /// APIs keyed by API ID.
    apis: DashMap<String, ApiRecord>,
    /// Domain names keyed by domain name.
    domain_names: DashMap<String, DomainNameRecord>,
    /// VPC links keyed by VPC link ID.
    vpc_links: DashMap<String, VpcLinkRecord>,
    /// Tags keyed by resource ARN.
    tags: DashMap<String, HashMap<String, String>>,
}

/// A single HTTP API.
#[derive(Debug, Clone)]
pub struct ApiRecord {
    /// Unique API identifier (10-char alphanumeric).
    pub api_id: String,
    /// Human-readable API name.
    pub name: String,
    /// Protocol type: HTTP or WEBSOCKET.
    pub protocol_type: ProtocolType,
    /// Route selection expression (default: "${request.method} ${request.path}").
    pub route_selection_expression: String,
    /// API key selection expression.
    pub api_key_selection_expression: Option<String>,
    /// CORS configuration.
    pub cors_configuration: Option<CorsConfiguration>,
    /// Description.
    pub description: String,
    /// Whether clients can invoke the API by using the default execute-api endpoint.
    pub disable_execute_api_endpoint: bool,
    /// API version.
    pub version: String,
    /// The generated API endpoint.
    pub api_endpoint: String,
    /// Routes keyed by route ID.
    pub routes: HashMap<String, RouteRecord>,
    /// Integrations keyed by integration ID.
    pub integrations: HashMap<String, IntegrationRecord>,
    /// Stages keyed by stage name.
    pub stages: HashMap<String, StageRecord>,
    /// Deployments keyed by deployment ID.
    pub deployments: HashMap<String, DeploymentRecord>,
    /// Authorizers keyed by authorizer ID.
    pub authorizers: HashMap<String, AuthorizerRecord>,
    /// Models keyed by model ID.
    pub models: HashMap<String, ModelRecord>,
    /// Creation timestamp (ISO-8601).
    pub created_date: String,
}

/// Protocol type for an API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolType {
    Http,
    WebSocket,
}

/// CORS configuration for an API.
#[derive(Debug, Clone)]
pub struct CorsConfiguration {
    /// Allowed origins (e.g., ["*"] or ["https://example.com"]).
    pub allow_origins: Vec<String>,
    /// Allowed HTTP methods (e.g., ["GET", "POST"]).
    pub allow_methods: Vec<String>,
    /// Allowed headers.
    pub allow_headers: Vec<String>,
    /// Exposed headers (returned to browser).
    pub expose_headers: Vec<String>,
    /// Max age in seconds for preflight cache.
    pub max_age: Option<i32>,
    /// Whether credentials are included.
    pub allow_credentials: bool,
}

/// A route within an API.
#[derive(Debug, Clone)]
pub struct RouteRecord {
    /// Unique route identifier.
    pub route_id: String,
    /// Route key: "{METHOD} {path}" or "$default".
    /// Examples: "GET /items/{id}", "POST /items", "$default", "ANY /proxy/{proxy+}".
    pub route_key: String,
    /// Target integration: "integrations/{integrationId}".
    pub target: Option<String>,
    /// Authorization type: NONE, JWT, AWS_IAM, CUSTOM.
    pub authorization_type: AuthorizationType,
    /// Authorizer ID (if authorization_type is JWT or CUSTOM).
    pub authorizer_id: Option<String>,
    /// Authorization scopes for JWT authorizer.
    pub authorization_scopes: Vec<String>,
    /// Whether API key is required.
    pub api_key_required: bool,
    /// Model selection expression.
    pub model_selection_expression: Option<String>,
    /// Operation name (for documentation).
    pub operation_name: Option<String>,
    /// Request models.
    pub request_models: HashMap<String, String>,
    /// Request parameters.
    pub request_parameters: HashMap<String, ParameterConstraints>,
    /// Route responses.
    pub route_responses: HashMap<String, RouteResponseRecord>,
}

/// Authorization type for a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationType {
    None,
    Jwt,
    AwsIam,
    Custom,
}

/// Parameter constraints for route request parameters.
#[derive(Debug, Clone)]
pub struct ParameterConstraints {
    pub required: bool,
}

/// A route response.
#[derive(Debug, Clone)]
pub struct RouteResponseRecord {
    /// Unique route response identifier.
    pub route_response_id: String,
    /// Route response key (e.g., "$default").
    pub route_response_key: String,
    /// Model selection expression.
    pub model_selection_expression: Option<String>,
    /// Response models.
    pub response_models: HashMap<String, String>,
    /// Response parameters.
    pub response_parameters: HashMap<String, ParameterConstraints>,
}

/// An integration connecting a route to a backend.
#[derive(Debug, Clone)]
pub struct IntegrationRecord {
    /// Unique integration identifier.
    pub integration_id: String,
    /// Integration type.
    pub integration_type: IntegrationType,
    /// Integration method (for HTTP/HTTP_PROXY).
    pub integration_method: Option<String>,
    /// Integration URI.
    /// For AWS_PROXY (Lambda): "arn:aws:lambda:{region}:{account}:function:{name}"
    /// For HTTP_PROXY: "https://backend.example.com/path"
    pub integration_uri: Option<String>,
    /// Connection type: INTERNET or VPC_LINK.
    pub connection_type: ConnectionType,
    /// Connection ID (VPC link ID for VPC_LINK connection type).
    pub connection_id: Option<String>,
    /// Content handling strategy.
    pub content_handling_strategy: Option<ContentHandlingStrategy>,
    /// Credentials ARN for the integration.
    pub credentials_arn: Option<String>,
    /// Description.
    pub description: String,
    /// Passthrough behavior.
    pub passthrough_behavior: Option<PassthroughBehavior>,
    /// Payload format version: "1.0" or "2.0".
    pub payload_format_version: String,
    /// Request parameters mapping.
    pub request_parameters: HashMap<String, String>,
    /// Request templates.
    pub request_templates: HashMap<String, String>,
    /// Response parameters mapping.
    pub response_parameters: HashMap<String, HashMap<String, String>>,
    /// Template selection expression.
    pub template_selection_expression: Option<String>,
    /// Timeout in milliseconds (50-30000, default 30000).
    pub timeout_in_millis: u32,
    /// TLS configuration.
    pub tls_config: Option<TlsConfig>,
    /// Integration subtype (for AWS service integrations).
    pub integration_subtype: Option<String>,
}

/// Integration type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationType {
    /// AWS Lambda proxy integration.
    AwsProxy,
    /// HTTP proxy integration.
    HttpProxy,
    /// HTTP integration (non-proxy).
    Http,
    /// Mock integration.
    Mock,
    /// AWS service integration.
    Aws,
}

/// Connection type for an integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionType {
    Internet,
    VpcLink,
}

/// Content handling strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentHandlingStrategy {
    ConvertToBinary,
    ConvertToText,
}

/// Passthrough behavior for non-matching content types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassthroughBehavior {
    WhenNoMatch,
    Never,
    WhenNoTemplates,
}

/// TLS configuration for an integration.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub server_name_to_verify: Option<String>,
}

/// A deployment snapshot of API configuration.
#[derive(Debug, Clone)]
pub struct DeploymentRecord {
    /// Unique deployment identifier.
    pub deployment_id: String,
    /// Description.
    pub description: String,
    /// Whether this deployment was auto-deployed.
    pub auto_deployed: bool,
    /// Deployment status.
    pub deployment_status: DeploymentStatus,
    /// Deployment status message.
    pub deployment_status_message: Option<String>,
    /// Creation timestamp.
    pub created_date: String,
}

/// Deployment status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentStatus {
    Pending,
    Failed,
    Deployed,
}

/// A stage configuration for an API.
#[derive(Debug, Clone)]
pub struct StageRecord {
    /// Stage name (e.g., "prod", "$default").
    pub stage_name: String,
    /// Associated deployment ID.
    pub deployment_id: Option<String>,
    /// Stage description.
    pub description: String,
    /// Whether auto-deploy is enabled.
    /// When true, changes to routes/integrations automatically create new deployments.
    pub auto_deploy: bool,
    /// Stage variables (key-value pairs).
    pub stage_variables: HashMap<String, String>,
    /// Default route settings.
    pub default_route_settings: Option<RouteSettings>,
    /// Per-route settings overrides.
    pub route_settings: HashMap<String, RouteSettings>,
    /// Access log settings.
    pub access_log_settings: Option<AccessLogSettings>,
    /// Client certificate ID.
    pub client_certificate_id: Option<String>,
    /// Creation timestamp.
    pub created_date: String,
    /// Last updated timestamp.
    pub last_updated_date: String,
}

/// Route-level settings for a stage.
#[derive(Debug, Clone)]
pub struct RouteSettings {
    /// Whether data trace logging is enabled.
    pub data_trace_enabled: bool,
    /// Whether detailed metrics are enabled.
    pub detailed_metrics_enabled: bool,
    /// Logging level: ERROR, INFO, OFF.
    pub logging_level: Option<String>,
    /// Throttling burst limit.
    pub throttling_burst_limit: Option<i32>,
    /// Throttling rate limit.
    pub throttling_rate_limit: Option<f64>,
}

/// Access log settings for a stage.
#[derive(Debug, Clone)]
pub struct AccessLogSettings {
    /// Destination ARN (e.g., CloudWatch Logs group ARN).
    pub destination_arn: Option<String>,
    /// Log format.
    pub format: Option<String>,
}

/// An authorizer for an API.
#[derive(Debug, Clone)]
pub struct AuthorizerRecord {
    /// Unique authorizer identifier.
    pub authorizer_id: String,
    /// Authorizer name.
    pub name: String,
    /// Authorizer type: JWT or REQUEST.
    pub authorizer_type: AuthorizerType,
    /// JWT configuration (for JWT type).
    pub jwt_configuration: Option<JwtConfiguration>,
    /// Authorizer credentials ARN (for REQUEST type).
    pub authorizer_credentials_arn: Option<String>,
    /// Authorizer URI (for REQUEST type -- Lambda function ARN).
    pub authorizer_uri: Option<String>,
    /// Identity source expression.
    pub identity_source: Option<String>,
    /// Authorizer payload format version.
    pub authorizer_payload_format_version: Option<String>,
    /// Authorizer result TTL in seconds.
    pub authorizer_result_ttl_in_seconds: Option<i32>,
    /// Whether the authorizer is enabled.
    pub enable_simple_responses: bool,
}

/// Authorizer type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizerType {
    Jwt,
    Request,
}

/// JWT configuration for a JWT authorizer.
#[derive(Debug, Clone)]
pub struct JwtConfiguration {
    /// The issuer URL (e.g., "https://cognito-idp.{region}.amazonaws.com/{userPoolId}").
    pub issuer: Option<String>,
    /// The list of audiences (client IDs).
    pub audience: Vec<String>,
}

/// A model schema for an API.
#[derive(Debug, Clone)]
pub struct ModelRecord {
    /// Unique model identifier.
    pub model_id: String,
    /// Model name.
    pub name: String,
    /// Content type.
    pub content_type: String,
    /// JSON schema string.
    pub schema: String,
    /// Description.
    pub description: String,
}

/// A custom domain name.
#[derive(Debug, Clone)]
pub struct DomainNameRecord {
    /// The domain name (e.g., "api.example.com").
    pub domain_name: String,
    /// Domain name configurations.
    pub domain_name_configurations: Vec<DomainNameConfiguration>,
    /// Mutual TLS authentication configuration.
    pub mutual_tls_authentication: Option<MutualTlsAuthentication>,
    /// API mappings for this domain name.
    pub api_mappings: HashMap<String, ApiMappingRecord>,
}

/// Domain name endpoint configuration.
#[derive(Debug, Clone)]
pub struct DomainNameConfiguration {
    /// API Gateway domain name.
    pub api_gateway_domain_name: Option<String>,
    /// Certificate ARN.
    pub certificate_arn: Option<String>,
    /// Certificate name.
    pub certificate_name: Option<String>,
    /// Certificate upload date.
    pub certificate_upload_date: Option<String>,
    /// Domain name status.
    pub domain_name_status: Option<String>,
    /// Domain name status message.
    pub domain_name_status_message: Option<String>,
    /// Endpoint type: REGIONAL or EDGE.
    pub endpoint_type: Option<String>,
    /// Hosted zone ID.
    pub hosted_zone_id: Option<String>,
    /// Security policy (e.g., "TLS_1_2").
    pub security_policy: Option<String>,
}

/// Mutual TLS authentication configuration.
#[derive(Debug, Clone)]
pub struct MutualTlsAuthentication {
    /// Truststore URI (S3 URI).
    pub truststore_uri: Option<String>,
    /// Truststore version.
    pub truststore_version: Option<String>,
}

/// An API mapping connecting a domain to an API stage.
#[derive(Debug, Clone)]
pub struct ApiMappingRecord {
    /// Unique API mapping identifier.
    pub api_mapping_id: String,
    /// API ID.
    pub api_id: String,
    /// API mapping key (path prefix).
    pub api_mapping_key: String,
    /// Stage name.
    pub stage: String,
}

/// A VPC link.
#[derive(Debug, Clone)]
pub struct VpcLinkRecord {
    /// Unique VPC link identifier.
    pub vpc_link_id: String,
    /// VPC link name.
    pub name: String,
    /// Security group IDs.
    pub security_group_ids: Vec<String>,
    /// Subnet IDs.
    pub subnet_ids: Vec<String>,
    /// VPC link status.
    pub vpc_link_status: VpcLinkStatus,
    /// VPC link status message.
    pub vpc_link_status_message: Option<String>,
    /// Creation timestamp.
    pub created_date: String,
}

/// VPC link status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VpcLinkStatus {
    Pending,
    Available,
    Deleting,
    Failed,
    Inactive,
}
```

### 9.3 Storage Layout

All API-scoped resources (routes, integrations, stages, deployments, authorizers, models) are stored within the `ApiRecord` as `HashMap`s. This provides:

- **Efficient lookup**: O(1) by entity ID within an API.
- **Atomic API deletion**: Deleting an API removes all child resources.
- **Natural hierarchy**: The storage mirrors the API Gateway v2 resource hierarchy.

Domain names and VPC links are top-level resources in separate `DashMap`s since they are not scoped to a specific API.

### 9.4 ID Generation

API Gateway v2 uses 10-character alphanumeric IDs for most resources:

```rust
/// Generate a random 10-character alphanumeric ID.
///
/// Matches the format used by AWS API Gateway v2 (e.g., "abc1234def").
fn generate_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect()
}
```

### 9.5 ARN Construction

```rust
fn api_arn(region: &str, account_id: &str, api_id: &str) -> String {
    format!("arn:aws:apigateway:{region}::/apis/{api_id}")
}

fn route_arn(region: &str, account_id: &str, api_id: &str, route_id: &str) -> String {
    format!("arn:aws:apigateway:{region}::/apis/{api_id}/routes/{route_id}")
}

fn stage_arn(region: &str, account_id: &str, api_id: &str, stage_name: &str) -> String {
    format!("arn:aws:apigateway:{region}::/apis/{api_id}/stages/{stage_name}")
}

fn integration_arn(region: &str, account_id: &str, api_id: &str, integration_id: &str) -> String {
    format!("arn:aws:apigateway:{region}::/apis/{api_id}/integrations/{integration_id}")
}
```

Note: API Gateway ARNs do not include the account ID in the resource path (they use `::` between region and resource).

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main API Gateway v2 provider. Owns resource storage and the execution engine.
pub struct RustackApiGatewayV2 {
    /// Resource storage (APIs, routes, integrations, stages, etc.).
    store: ApiStore,
    /// Configuration.
    config: Arc<ApiGatewayV2Config>,
    /// Lambda provider for Lambda proxy integrations (optional).
    #[cfg(feature = "lambda-integration")]
    lambda: Option<Arc<RustackLambda>>,
    /// HTTP client for HTTP proxy integrations.
    http_client: reqwest::Client,
}

pub struct ApiGatewayV2Config {
    pub skip_signature_validation: bool,
    pub default_region: String,
    pub account_id: String,
    pub host: String,
    pub port: u16,
}

impl ApiGatewayV2Config {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("APIGATEWAYV2_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
        }
    }
}
```

### 10.2 Operations Grouped by Phase

#### Phase 0: API, Route, Integration Management (15 operations)

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `CreateApi` | Medium | Generate API ID, set defaults (route selection expression, protocol type), construct endpoint URL |
| `GetApi` | Low | Lookup by API ID |
| `UpdateApi` | Low | Partial update (PATCH semantics -- only update provided fields) |
| `DeleteApi` | Medium | Cascade delete all child resources (routes, integrations, stages, etc.) |
| `GetApis` | Low | Paginate with NextToken/MaxResults |
| `CreateRoute` | Medium | Parse route key ("METHOD /path"), validate integration target exists, handle `$default` route |
| `GetRoute` | Low | Lookup by API ID + route ID |
| `UpdateRoute` | Low | Partial update |
| `DeleteRoute` | Low | Remove route, delete associated route responses |
| `GetRoutes` | Low | List all routes for an API |
| `CreateIntegration` | Medium | Validate integration type, URI format, set payload format version defaults |
| `GetIntegration` | Low | Lookup by API ID + integration ID |
| `UpdateIntegration` | Low | Partial update |
| `DeleteIntegration` | Medium | Remove integration, clear route targets pointing to this integration |
| `GetIntegrations` | Low | List all integrations for an API |

#### Phase 1: Stages, Deployments, Route Responses (12 operations)

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `CreateStage` | Medium | Handle `$default` stage, auto-deploy flag, stage variables |
| `GetStage` | Low | Lookup by API ID + stage name |
| `UpdateStage` | Medium | Partial update, trigger auto-deployment if enabled |
| `DeleteStage` | Low | Remove stage |
| `GetStages` | Low | List all stages for an API |
| `CreateDeployment` | Medium | Snapshot current API configuration as a deployment |
| `GetDeployment` | Low | Lookup by API ID + deployment ID |
| `DeleteDeployment` | Low | Remove deployment, clear stage references |
| `GetDeployments` | Low | List all deployments for an API |
| `CreateRouteResponse` | Low | Add response to a route |
| `GetRouteResponse` | Low | Lookup by API ID + route ID + route response ID |
| `DeleteRouteResponse` | Low | Remove route response |

#### Phase 2: Authorizers, Models (11 operations)

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `CreateAuthorizer` | Medium | Validate JWT configuration (issuer, audience), or REQUEST authorizer (Lambda URI) |
| `GetAuthorizer` | Low | Lookup by API ID + authorizer ID |
| `UpdateAuthorizer` | Low | Partial update |
| `DeleteAuthorizer` | Low | Remove authorizer, clear route references |
| `GetAuthorizers` | Low | List all authorizers for an API |
| `CreateModel` | Low | Store JSON schema |
| `GetModel` | Low | Lookup by API ID + model ID |
| `UpdateModel` | Low | Partial update |
| `DeleteModel` | Low | Remove model |
| `GetModels` | Low | List all models for an API |
| `GetModelTemplate` | Low | Return schema as template |

#### Phase 3: Domain Names, VPC Links, Tags, API Mappings (18 operations)

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `CreateDomainName` | Low | Store domain configuration (no TLS provisioning) |
| `GetDomainName` | Low | Lookup by domain name |
| `UpdateDomainName` | Low | Partial update |
| `DeleteDomainName` | Low | Remove domain, cascade delete API mappings |
| `GetDomainNames` | Low | List all domain names |
| `CreateVpcLink` | Low | Store VPC link configuration (no actual VPC connectivity) |
| `GetVpcLink` | Low | Lookup by VPC link ID |
| `UpdateVpcLink` | Low | Partial update |
| `DeleteVpcLink` | Low | Remove VPC link |
| `GetVpcLinks` | Low | List all VPC links |
| `TagResource` | Low | Add/update tags on any API GW resource |
| `UntagResource` | Low | Remove tags by key |
| `GetTags` | Low | Return all tags for a resource |
| `CreateApiMapping` | Low | Map domain name path to API + stage |
| `GetApiMapping` | Low | Lookup by domain name + mapping ID |
| `UpdateApiMapping` | Low | Partial update |
| `DeleteApiMapping` | Low | Remove API mapping |
| `GetApiMappings` | Low | List all API mappings for a domain |

#### Phase 4: Execution Engine

| Component | Complexity | Notes |
|-----------|------------|-------|
| Route matching | High | Parse route keys, match incoming requests against configured routes with path parameters, greedy parameters, and `$default` fallback |
| Lambda proxy integration | High | Build API Gateway v2 event payload, invoke Lambda via `rustack-lambda-core`, parse Lambda response |
| HTTP proxy integration | Medium | Forward request to backend URL, return response |
| Mock integration | Low | Return static configured response |
| CORS handling | Medium | Generate preflight responses, add CORS headers to responses |
| Auto-deploy | Medium | Detect route/integration changes, auto-create deployments when stage has `auto_deploy` enabled |

### 10.3 CreateApi Logic

```rust
impl RustackApiGatewayV2 {
    pub fn create_api(
        &self,
        input: CreateApiInput,
    ) -> Result<CreateApiOutput, ApiGatewayV2ServiceError> {
        let api_id = generate_id();

        // Validate protocol type.
        let protocol_type = match input.protocol_type.as_deref() {
            Some("HTTP") | None => ProtocolType::Http,
            Some("WEBSOCKET") => ProtocolType::WebSocket,
            Some(other) => {
                return Err(ApiGatewayV2ServiceError::BadRequest {
                    message: format!("Invalid protocol type: {other}"),
                });
            }
        };

        // Build CORS configuration if provided.
        let cors_configuration = input.cors_configuration.map(|cors| CorsConfiguration {
            allow_origins: cors.allow_origins.unwrap_or_default(),
            allow_methods: cors.allow_methods.unwrap_or_default(),
            allow_headers: cors.allow_headers.unwrap_or_default(),
            expose_headers: cors.expose_headers.unwrap_or_default(),
            max_age: cors.max_age,
            allow_credentials: cors.allow_credentials.unwrap_or(false),
        });

        let now = chrono::Utc::now().to_rfc3339();
        let api_endpoint = format!(
            "http://{}:{}/_aws/execute-api/{}",
            self.config.host, self.config.port, api_id,
        );

        let route_selection_expression = input
            .route_selection_expression
            .unwrap_or_else(|| "${request.method} ${request.path}".to_string());

        let api = ApiRecord {
            api_id: api_id.clone(),
            name: input.name.clone(),
            protocol_type,
            route_selection_expression,
            api_key_selection_expression: input.api_key_selection_expression,
            cors_configuration,
            description: input.description.unwrap_or_default(),
            disable_execute_api_endpoint: input.disable_execute_api_endpoint.unwrap_or(false),
            version: input.version.unwrap_or_default(),
            api_endpoint,
            routes: HashMap::new(),
            integrations: HashMap::new(),
            stages: HashMap::new(),
            deployments: HashMap::new(),
            authorizers: HashMap::new(),
            models: HashMap::new(),
            created_date: now,
        };

        let output = self.api_to_output(&api);
        self.store.apis.insert(api_id, api);
        Ok(output)
    }
}
```

### 10.4 CreateRoute Logic

```rust
impl RustackApiGatewayV2 {
    pub fn create_route(
        &self,
        api_id: &str,
        input: CreateRouteInput,
    ) -> Result<CreateRouteOutput, ApiGatewayV2ServiceError> {
        let mut api = self.store.apis.get_mut(api_id)
            .ok_or_else(|| ApiGatewayV2ServiceError::NotFound {
                message: format!("Invalid API identifier specified {api_id}"),
            })?;

        let route_key = &input.route_key;

        // Validate route key format: "METHOD /path" or "$default".
        if route_key != "$default" {
            let parts: Vec<&str> = route_key.splitn(2, ' ').collect();
            if parts.len() != 2 {
                return Err(ApiGatewayV2ServiceError::BadRequest {
                    message: format!("Invalid route key: {route_key}"),
                });
            }
            let method = parts[0];
            let valid_methods = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "ANY"];
            if !valid_methods.contains(&method) {
                return Err(ApiGatewayV2ServiceError::BadRequest {
                    message: format!("Invalid HTTP method in route key: {method}"),
                });
            }
        }

        // Validate target integration exists if provided.
        if let Some(ref target) = input.target {
            if let Some(integration_id) = target.strip_prefix("integrations/") {
                if !api.integrations.contains_key(integration_id) {
                    return Err(ApiGatewayV2ServiceError::NotFound {
                        message: format!("Invalid integration identifier specified {integration_id}"),
                    });
                }
            }
        }

        // Check for duplicate route key.
        for existing in api.routes.values() {
            if existing.route_key == *route_key {
                return Err(ApiGatewayV2ServiceError::Conflict {
                    message: format!("Route with key {route_key} already exists"),
                });
            }
        }

        let route_id = generate_id();
        let route = RouteRecord {
            route_id: route_id.clone(),
            route_key: route_key.clone(),
            target: input.target.clone(),
            authorization_type: match input.authorization_type.as_deref() {
                Some("JWT") => AuthorizationType::Jwt,
                Some("AWS_IAM") => AuthorizationType::AwsIam,
                Some("CUSTOM") => AuthorizationType::Custom,
                _ => AuthorizationType::None,
            },
            authorizer_id: input.authorizer_id.clone(),
            authorization_scopes: input.authorization_scopes.unwrap_or_default(),
            api_key_required: input.api_key_required.unwrap_or(false),
            model_selection_expression: input.model_selection_expression.clone(),
            operation_name: input.operation_name.clone(),
            request_models: input.request_models.unwrap_or_default(),
            request_parameters: input.request_parameters.unwrap_or_default()
                .into_iter()
                .map(|(k, v)| (k, ParameterConstraints { required: v.required }))
                .collect(),
            route_responses: HashMap::new(),
        };

        let output = self.route_to_output(api_id, &route);
        api.routes.insert(route_id, route);

        // Trigger auto-deploy if any stage has it enabled.
        self.maybe_auto_deploy(&mut api);

        Ok(output)
    }
}
```

### 10.5 CreateIntegration Logic

```rust
impl RustackApiGatewayV2 {
    pub fn create_integration(
        &self,
        api_id: &str,
        input: CreateIntegrationInput,
    ) -> Result<CreateIntegrationOutput, ApiGatewayV2ServiceError> {
        let mut api = self.store.apis.get_mut(api_id)
            .ok_or_else(|| ApiGatewayV2ServiceError::NotFound {
                message: format!("Invalid API identifier specified {api_id}"),
            })?;

        let integration_type = match input.integration_type.as_deref() {
            Some("AWS_PROXY") => IntegrationType::AwsProxy,
            Some("HTTP_PROXY") => IntegrationType::HttpProxy,
            Some("HTTP") => IntegrationType::Http,
            Some("MOCK") => IntegrationType::Mock,
            Some("AWS") => IntegrationType::Aws,
            Some(other) => {
                return Err(ApiGatewayV2ServiceError::BadRequest {
                    message: format!("Invalid integration type: {other}"),
                });
            }
            None => {
                return Err(ApiGatewayV2ServiceError::BadRequest {
                    message: "integrationType is required".to_string(),
                });
            }
        };

        // Validate integration URI based on type.
        match integration_type {
            IntegrationType::AwsProxy => {
                if let Some(ref uri) = input.integration_uri {
                    if !uri.starts_with("arn:aws:") {
                        return Err(ApiGatewayV2ServiceError::BadRequest {
                            message: "AWS_PROXY integration URI must be an ARN".to_string(),
                        });
                    }
                }
            }
            IntegrationType::HttpProxy | IntegrationType::Http => {
                if input.integration_uri.is_none() {
                    return Err(ApiGatewayV2ServiceError::BadRequest {
                        message: "integrationUri is required for HTTP/HTTP_PROXY integrations"
                            .to_string(),
                    });
                }
            }
            IntegrationType::Mock | IntegrationType::Aws => {}
        }

        let integration_id = generate_id();
        let integration = IntegrationRecord {
            integration_id: integration_id.clone(),
            integration_type,
            integration_method: input.integration_method.clone(),
            integration_uri: input.integration_uri.clone(),
            connection_type: match input.connection_type.as_deref() {
                Some("VPC_LINK") => ConnectionType::VpcLink,
                _ => ConnectionType::Internet,
            },
            connection_id: input.connection_id.clone(),
            content_handling_strategy: input.content_handling_strategy.as_deref().and_then(|s| {
                match s {
                    "CONVERT_TO_BINARY" => Some(ContentHandlingStrategy::ConvertToBinary),
                    "CONVERT_TO_TEXT" => Some(ContentHandlingStrategy::ConvertToText),
                    _ => None,
                }
            }),
            credentials_arn: input.credentials_arn.clone(),
            description: input.description.unwrap_or_default(),
            passthrough_behavior: input.passthrough_behavior.as_deref().and_then(|s| match s {
                "WHEN_NO_MATCH" => Some(PassthroughBehavior::WhenNoMatch),
                "NEVER" => Some(PassthroughBehavior::Never),
                "WHEN_NO_TEMPLATES" => Some(PassthroughBehavior::WhenNoTemplates),
                _ => None,
            }),
            payload_format_version: input
                .payload_format_version
                .unwrap_or_else(|| "1.0".to_string()),
            request_parameters: input.request_parameters.unwrap_or_default(),
            request_templates: input.request_templates.unwrap_or_default(),
            response_parameters: input.response_parameters.unwrap_or_default(),
            template_selection_expression: input.template_selection_expression.clone(),
            timeout_in_millis: input.timeout_in_millis.unwrap_or(30_000),
            tls_config: input.tls_config.map(|t| TlsConfig {
                server_name_to_verify: t.server_name_to_verify,
            }),
            integration_subtype: input.integration_subtype.clone(),
        };

        let output = self.integration_to_output(api_id, &integration);
        api.integrations.insert(integration_id, integration);

        // Trigger auto-deploy if any stage has it enabled.
        self.maybe_auto_deploy(&mut api);

        Ok(output)
    }
}
```

### 10.6 Auto-Deploy Logic

```rust
impl RustackApiGatewayV2 {
    /// Check if any stage has auto_deploy enabled and create a deployment if so.
    fn maybe_auto_deploy(&self, api: &mut ApiRecord) {
        let auto_deploy_stages: Vec<String> = api
            .stages
            .values()
            .filter(|s| s.auto_deploy)
            .map(|s| s.stage_name.clone())
            .collect();

        if auto_deploy_stages.is_empty() {
            return;
        }

        let deployment_id = generate_id();
        let now = chrono::Utc::now().to_rfc3339();
        let deployment = DeploymentRecord {
            deployment_id: deployment_id.clone(),
            description: String::new(),
            auto_deployed: true,
            deployment_status: DeploymentStatus::Deployed,
            deployment_status_message: None,
            created_date: now.clone(),
        };
        api.deployments.insert(deployment_id.clone(), deployment);

        // Update each auto-deploy stage to point to the new deployment.
        for stage_name in auto_deploy_stages {
            if let Some(stage) = api.stages.get_mut(&stage_name) {
                stage.deployment_id = Some(deployment_id.clone());
                stage.last_updated_date = now.clone();
            }
        }
    }
}
```

### 10.7 Execution Engine

The execution engine is the most complex component. It handles incoming HTTP requests and routes them through configured API Gateway resources.

#### 10.7.1 Execution Flow

```
    HTTP request arrives at /_aws/execute-api/{apiId}/{stage}/{path}
            |
            v
    +-------------------+
    | Parse execution   |  Extract apiId, stage, proxy path
    | target from URL   |
    +--------+----------+
             |
             v
    +-------------------+
    | Look up API and   |  Verify API exists, stage exists,
    | stage             |  stage has a deployment
    +--------+----------+
             |
             v
    +-------------------+
    | CORS preflight?   |  If OPTIONS with Origin header,
    | check             |  return CORS preflight response
    +--------+----------+
            / \
        yes/   \no
          /     \
         v       v
  +----------+  +-------------------+
  | Return   |  | Match route       |  Match "{METHOD} {path}" against
  | preflight|  | against configured|  route keys, try exact then
  | response |  | routes            |  parameterized then $default
  +----------+  +--------+----------+
                         |
                         v
                +-------------------+
                | Resolve target    |  Route target -> IntegrationRecord
                | integration       |
                +--------+----------+
                         |
                         v
                +-------------------+
                | Dispatch to       |  Based on integration_type:
                | integration       |  AWS_PROXY, HTTP_PROXY, MOCK
                +--------+----------+
                        / | \
                       /  |  \
                      /   |   \
                     v    v    v
              +------+ +-----+ +------+
              |Lambda| |HTTP | |Mock  |
              |Proxy | |Proxy| |      |
              +------+ +-----+ +------+
                     \    |    /
                      \   |   /
                       v  v  v
                +-------------------+
                | Add CORS headers  |  If API has cors_configuration,
                | to response       |  add Access-Control-* headers
                +--------+----------+
                         |
                         v
                    HTTP response
```

#### 10.7.2 Route Matching Algorithm

```rust
/// Match an incoming request against configured API routes.
///
/// Matching priority (from highest to lowest):
/// 1. Exact match: route key matches exactly (e.g., "GET /items" matches "GET /items")
/// 2. Parameterized match: path template matches (e.g., "GET /items/{id}" matches "GET /items/42")
/// 3. Greedy match: greedy path parameter (e.g., "GET /proxy/{proxy+}" matches "GET /proxy/a/b/c")
/// 4. ANY method match: "ANY /path" matches any HTTP method
/// 5. $default route: catch-all fallback
pub fn match_route<'a>(
    method: &http::Method,
    path: &str,
    routes: &'a HashMap<String, RouteRecord>,
) -> Option<(&'a RouteRecord, HashMap<String, String>)> {
    let method_str = method.as_str();
    let mut best_match: Option<(&RouteRecord, HashMap<String, String>, MatchPriority)> = None;

    for route in routes.values() {
        if route.route_key == "$default" {
            // $default is the lowest priority fallback.
            if best_match.is_none() {
                best_match = Some((route, HashMap::new(), MatchPriority::Default));
            }
            continue;
        }

        let (route_method, route_path) = match route.route_key.split_once(' ') {
            Some((m, p)) => (m, p),
            None => continue,
        };

        // Check method match.
        let method_matches = route_method == method_str || route_method == "ANY";
        if !method_matches {
            continue;
        }

        // Try exact path match.
        if route_path == path {
            let priority = if route_method == "ANY" {
                MatchPriority::AnyExact
            } else {
                MatchPriority::Exact
            };
            if best_match.as_ref().map_or(true, |(_, _, p)| priority > *p) {
                best_match = Some((route, HashMap::new(), priority));
            }
            continue;
        }

        // Try parameterized path match.
        if let Some(params) = match_path_template(path, route_path) {
            let has_greedy = route_path.contains("{proxy+}") || route_path.contains('+');
            let priority = if has_greedy {
                if route_method == "ANY" {
                    MatchPriority::AnyGreedy
                } else {
                    MatchPriority::Greedy
                }
            } else if route_method == "ANY" {
                MatchPriority::AnyParameterized
            } else {
                MatchPriority::Parameterized
            };
            if best_match.as_ref().map_or(true, |(_, _, p)| priority > *p) {
                best_match = Some((route, params, priority));
            }
        }
    }

    best_match.map(|(route, params, _)| (route, params))
}

/// Match priority ordering (highest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MatchPriority {
    Default = 0,
    AnyGreedy = 1,
    Greedy = 2,
    AnyParameterized = 3,
    Parameterized = 4,
    AnyExact = 5,
    Exact = 6,
}

/// Match a request path against a route path template.
///
/// Template: "/items/{id}" matches path: "/items/42" -> {"id": "42"}
/// Template: "/proxy/{proxy+}" matches path: "/proxy/a/b/c" -> {"proxy": "a/b/c"}
fn match_path_template(
    path: &str,
    template: &str,
) -> Option<HashMap<String, String>> {
    let path_segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let tmpl_segments: Vec<&str> = template.trim_matches('/').split('/').collect();

    let mut params = HashMap::new();
    let mut pi = 0;
    let mut ti = 0;

    while ti < tmpl_segments.len() {
        let tmpl_seg = tmpl_segments[ti];

        if tmpl_seg.starts_with('{') && tmpl_seg.ends_with('}') {
            let param_name = &tmpl_seg[1..tmpl_seg.len() - 1];

            if param_name.ends_with('+') {
                // Greedy parameter: captures remaining path segments.
                let clean_name = &param_name[..param_name.len() - 1];
                if pi >= path_segments.len() {
                    return None;
                }
                let remaining: Vec<&str> = path_segments[pi..].to_vec();
                params.insert(clean_name.to_string(), remaining.join("/"));
                return Some(params);
            }

            // Standard parameter: captures a single segment.
            if pi >= path_segments.len() {
                return None;
            }
            params.insert(param_name.to_string(), path_segments[pi].to_string());
        } else {
            // Literal segment: must match exactly.
            if pi >= path_segments.len() || path_segments[pi] != tmpl_seg {
                return None;
            }
        }

        pi += 1;
        ti += 1;
    }

    // Both path and template must be fully consumed (unless greedy).
    if pi == path_segments.len() {
        Some(params)
    } else {
        None
    }
}
```

#### 10.7.3 Lambda Proxy Integration

When integration_type is `AWS_PROXY` and the integration_uri is a Lambda function ARN, the execution engine builds an API Gateway v2 event payload and invokes the Lambda function.

```rust
/// Build an API Gateway v2 payload format 2.0 event from the incoming request.
///
/// Reference: https://docs.aws.amazon.com/apigateway/latest/developerguide/http-api-develop-integrations-lambda.html
fn build_lambda_event_v2(
    method: &http::Method,
    path: &str,
    headers: &http::HeaderMap,
    query: &str,
    body: &[u8],
    path_parameters: &HashMap<String, String>,
    stage: &str,
    route_key: &str,
    api_id: &str,
    account_id: &str,
    region: &str,
) -> serde_json::Value {
    let is_base64 = !body.is_empty() && !is_text_content(headers);
    let body_str = if is_base64 {
        Some(base64::engine::general_purpose::STANDARD.encode(body))
    } else if body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(body).into_owned())
    };

    let headers_map: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                serde_json::Value::String(v.to_str().unwrap_or("").to_string()),
            )
        })
        .collect();

    let query_params = parse_query_string(query);

    let now = chrono::Utc::now();
    serde_json::json!({
        "version": "2.0",
        "routeKey": route_key,
        "rawPath": path,
        "rawQueryString": query,
        "headers": headers_map,
        "queryStringParameters": query_params,
        "pathParameters": path_parameters,
        "requestContext": {
            "accountId": account_id,
            "apiId": api_id,
            "domainName": format!("{api_id}.execute-api.{region}.amazonaws.com"),
            "domainPrefix": api_id,
            "http": {
                "method": method.as_str(),
                "path": path,
                "protocol": "HTTP/1.1",
                "sourceIp": "127.0.0.1",
                "userAgent": headers.get("user-agent")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
            },
            "requestId": uuid::Uuid::new_v4().to_string(),
            "routeKey": route_key,
            "stage": stage,
            "time": now.format("%d/%b/%Y:%H:%M:%S %z").to_string(),
            "timeEpoch": now.timestamp_millis(),
        },
        "body": body_str,
        "isBase64Encoded": is_base64,
    })
}

/// Parse a Lambda proxy integration response back to HTTP.
///
/// Lambda can return either:
/// - Format 2.0: { statusCode, headers, body, isBase64Encoded, cookies }
/// - Simple string: treated as 200 with text body
fn parse_lambda_response(
    payload: &[u8],
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| ApiGatewayV2ServiceError::IntegrationError {
            message: format!("Failed to parse Lambda response: {e}"),
        })?;

    // If the response has a statusCode field, it's format 2.0.
    if let Some(status_code) = value.get("statusCode").and_then(|v| v.as_u64()) {
        let status = http::StatusCode::from_u16(status_code as u16)
            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);

        let mut builder = http::Response::builder().status(status);

        // Add headers from response.
        if let Some(headers) = value.get("headers").and_then(|v| v.as_object()) {
            for (key, val) in headers {
                if let Some(val_str) = val.as_str() {
                    builder = builder.header(key.as_str(), val_str);
                }
            }
        }

        // Add cookies.
        if let Some(cookies) = value.get("cookies").and_then(|v| v.as_array()) {
            for cookie in cookies {
                if let Some(cookie_str) = cookie.as_str() {
                    builder = builder.header("set-cookie", cookie_str);
                }
            }
        }

        // Decode body.
        let is_base64 = value
            .get("isBase64Encoded")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let body = match value.get("body") {
            Some(serde_json::Value::String(s)) => {
                if is_base64 {
                    Bytes::from(
                        base64::engine::general_purpose::STANDARD
                            .decode(s)
                            .unwrap_or_default(),
                    )
                } else {
                    Bytes::from(s.clone())
                }
            }
            Some(v) => Bytes::from(v.to_string()),
            None => Bytes::new(),
        };

        builder
            .body(body)
            .map_err(|e| ApiGatewayV2ServiceError::IntegrationError {
                message: format!("Failed to build response: {e}"),
            })
    } else {
        // Simple string response: wrap in 200 OK.
        Ok(http::Response::builder()
            .status(http::StatusCode::OK)
            .header("content-type", "application/json")
            .body(Bytes::copy_from_slice(payload))
            .expect("simple response should be valid"))
    }
}
```

#### 10.7.4 HTTP Proxy Integration

```rust
/// Forward a request to a backend HTTP URL.
async fn execute_http_proxy(
    client: &reqwest::Client,
    integration: &IntegrationRecord,
    method: &http::Method,
    path: &str,
    headers: &http::HeaderMap,
    query: &str,
    body: Bytes,
    path_parameters: &HashMap<String, String>,
    stage_variables: &HashMap<String, String>,
) -> Result<http::Response<Bytes>, ApiGatewayV2ServiceError> {
    let base_uri = integration.integration_uri.as_deref()
        .ok_or_else(|| ApiGatewayV2ServiceError::IntegrationError {
            message: "Integration URI is required for HTTP_PROXY".to_string(),
        })?;

    // Substitute path parameters and stage variables in the URI.
    let mut uri = base_uri.to_string();
    for (key, value) in path_parameters {
        uri = uri.replace(&format!("{{{key}}}"), value);
    }
    for (key, value) in stage_variables {
        uri = uri.replace(&format!("${{stageVariables.{key}}}"), value);
    }

    // Append the proxy path if not already included.
    if !uri.contains(path) {
        uri = format!("{}{}", uri.trim_end_matches('/'), path);
    }

    // Append query string.
    if !query.is_empty() {
        uri = format!("{uri}?{query}");
    }

    let req_method = match integration.integration_method.as_deref() {
        Some(m) => m.parse().unwrap_or(reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET)),
        None => reqwest::Method::from_bytes(method.as_str().as_bytes())
            .unwrap_or(reqwest::Method::GET),
    };

    let mut req = client.request(req_method, &uri);

    // Forward headers (excluding host and connection).
    for (key, value) in headers {
        let name = key.as_str();
        if name != "host" && name != "connection" {
            if let Ok(v) = value.to_str() {
                req = req.header(name, v);
            }
        }
    }

    if !body.is_empty() {
        req = req.body(body);
    }

    let timeout = Duration::from_millis(integration.timeout_in_millis as u64);
    let resp = req
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| ApiGatewayV2ServiceError::IntegrationError {
            message: format!("HTTP proxy request failed: {e}"),
        })?;

    let status = http::StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(http::StatusCode::BAD_GATEWAY);

    let mut builder = http::Response::builder().status(status);
    for (key, value) in resp.headers() {
        builder = builder.header(key.as_str(), value.as_bytes());
    }

    let resp_body = resp.bytes().await
        .map_err(|e| ApiGatewayV2ServiceError::IntegrationError {
            message: format!("Failed to read backend response: {e}"),
        })?;

    builder
        .body(Bytes::from(resp_body))
        .map_err(|e| ApiGatewayV2ServiceError::IntegrationError {
            message: format!("Failed to build response: {e}"),
        })
}
```

#### 10.7.5 CORS Handling

```rust
/// Handle CORS for API Gateway v2 HTTP APIs.
///
/// If the API has a cors_configuration:
/// - OPTIONS requests with an Origin header get an automatic preflight response
/// - All other responses get CORS headers added
pub fn handle_cors_preflight(
    api: &ApiRecord,
    method: &http::Method,
    headers: &http::HeaderMap,
) -> Option<http::Response<Bytes>> {
    let cors = api.cors_configuration.as_ref()?;

    // Only handle OPTIONS preflight requests.
    if *method != http::Method::OPTIONS {
        return None;
    }

    let origin = headers.get("origin")?.to_str().ok()?;

    // Check if origin is allowed.
    let origin_allowed = cors.allow_origins.iter().any(|o| o == "*" || o == origin);
    if !origin_allowed {
        return None;
    }

    let mut builder = http::Response::builder().status(http::StatusCode::NO_CONTENT);

    builder = builder.header("access-control-allow-origin", origin);

    if !cors.allow_methods.is_empty() {
        builder = builder.header(
            "access-control-allow-methods",
            cors.allow_methods.join(", "),
        );
    }

    if !cors.allow_headers.is_empty() {
        builder = builder.header(
            "access-control-allow-headers",
            cors.allow_headers.join(", "),
        );
    }

    if !cors.expose_headers.is_empty() {
        builder = builder.header(
            "access-control-expose-headers",
            cors.expose_headers.join(", "),
        );
    }

    if let Some(max_age) = cors.max_age {
        builder = builder.header("access-control-max-age", max_age.to_string());
    }

    if cors.allow_credentials {
        builder = builder.header("access-control-allow-credentials", "true");
    }

    Some(
        builder
            .body(Bytes::new())
            .expect("CORS preflight response should be valid"),
    )
}

/// Add CORS headers to a response based on API cors_configuration.
pub fn add_cors_headers(
    api: &ApiRecord,
    request_headers: &http::HeaderMap,
    response: &mut http::Response<Bytes>,
) {
    let cors = match api.cors_configuration.as_ref() {
        Some(c) => c,
        None => return,
    };

    let origin = match request_headers.get("origin").and_then(|v| v.to_str().ok()) {
        Some(o) => o,
        None => return,
    };

    let origin_allowed = cors.allow_origins.iter().any(|o| o == "*" || o == origin);
    if !origin_allowed {
        return;
    }

    let headers = response.headers_mut();
    headers.insert(
        "access-control-allow-origin",
        origin.parse().expect("origin is valid header value"),
    );

    if cors.allow_credentials {
        headers.insert("access-control-allow-credentials", "true".parse().unwrap());
    }

    if !cors.expose_headers.is_empty() {
        headers.insert(
            "access-control-expose-headers",
            cors.expose_headers.join(", ").parse().unwrap(),
        );
    }
}
```

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// API Gateway v2 service errors with HTTP status codes and error type strings.
#[derive(Debug, thiserror::Error)]
pub enum ApiGatewayV2ServiceError {
    #[error("The resource specified in the request was not found: {message}")]
    NotFound { message: String },

    #[error("A resource with the same identifier already exists: {message}")]
    Conflict { message: String },

    #[error("The request is not valid: {message}")]
    BadRequest { message: String },

    #[error("The number of requests exceeds the limit")]
    TooManyRequests,

    #[error("Access denied: {message}")]
    AccessDenied { message: String },

    #[error("Integration error: {message}")]
    IntegrationError { message: String },

    #[error("Internal server error: {message}")]
    Internal { message: String },
}
```

### 11.2 Error Mapping

```rust
impl ApiGatewayV2ServiceError {
    /// Map to (HTTP status, error type string, message).
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::NotFound { .. } =>
                (404, "NotFoundException", self.to_string()),
            Self::Conflict { .. } =>
                (409, "ConflictException", self.to_string()),
            Self::BadRequest { .. } =>
                (400, "BadRequestException", self.to_string()),
            Self::TooManyRequests =>
                (429, "TooManyRequestsException", self.to_string()),
            Self::AccessDenied { .. } =>
                (403, "AccessDeniedException", self.to_string()),
            Self::IntegrationError { .. } =>
                (502, "ServiceUnavailableException", self.to_string()),
            Self::Internal { .. } =>
                (500, "InternalServerErrorException", self.to_string()),
        }
    }
}
```

### 11.3 Error Response Format

```rust
/// Format an API Gateway v2 error response for restJson1 protocol.
fn error_response(error: &ApiGatewayV2ServiceError) -> http::Response<Bytes> {
    let (status, error_type, message) = error.to_error_response();

    let body = serde_json::json!({
        "message": message,
    });

    http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("X-Amzn-Errortype", error_type)
        .body(Bytes::from(serde_json::to_vec(&body).expect("JSON serialization")))
        .expect("valid error response")
}
```

Note: API Gateway v2 uses lowercase `message` in error responses (not `Message` like Lambda or `__type` like DynamoDB).

---

## 12. Server Integration

### 12.1 Feature Gate

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns", "lambda", "events", "logs", "kms", "kinesis", "secretsmanager", "apigatewayv2"]
apigatewayv2 = ["dep:rustack-apigatewayv2-core", "dep:rustack-apigatewayv2-http"]
```

### 12.2 TWO ServiceRouter Registrations

API Gateway v2 needs two separate `ServiceRouter` implementations:

```rust
/// Routes management API requests to the API Gateway v2 service.
///
/// Matches requests whose URL path starts with /v2/apis, /v2/domainnames,
/// /v2/vpclinks, or /v2/tags.
pub struct ApiGatewayV2ManagementRouter<H: ApiGatewayV2Handler> {
    inner: ApiGatewayV2HttpService<H>,
}

impl<H: ApiGatewayV2Handler> ServiceRouter for ApiGatewayV2ManagementRouter<H> {
    fn name(&self) -> &'static str { "apigatewayv2" }

    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        let path = req.uri().path();
        path.starts_with("/v2/apis")
            || path.starts_with("/v2/domainnames")
            || path.starts_with("/v2/vpclinks")
            || path.starts_with("/v2/tags")
    }

    fn call(&self, req: http::Request<Incoming>)
        -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
    {
        let svc = self.inner.clone();
        Box::pin(async move {
            let resp = svc.call(req).await;
            Ok(resp.unwrap_or_else(|e| match e {}).map(BodyExt::boxed))
        })
    }
}

/// Routes execution requests through configured API Gateway v2 APIs.
///
/// Matches requests whose URL path starts with /_aws/execute-api/.
pub struct ApiGatewayV2ExecutionRouter<E: ExecutionHandler> {
    inner: E,
}

impl<E: ExecutionHandler> ServiceRouter for ApiGatewayV2ExecutionRouter<E> {
    fn name(&self) -> &'static str { "apigatewayv2-execute" }

    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        req.uri().path().starts_with("/_aws/execute-api/")
    }

    fn call(&self, req: http::Request<Incoming>)
        -> Pin<Box<dyn Future<Output = Result<http::Response<GatewayBody>, Infallible>> + Send>>
    {
        // Parse execution target from URL.
        // Extract method, headers, query, body from request.
        // Delegate to ExecutionHandler.
        // ...
    }
}
```

### 12.3 Gateway Registration Order

Both API Gateway v2 routers are registered before S3 (catch-all). The management router and execution router use distinct, non-overlapping URL prefixes:

```rust
fn build_gateway(config: &ServerConfig) -> GatewayService {
    let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

    // API Gateway v2 management API (path prefix: /v2/)
    #[cfg(feature = "apigatewayv2")]
    services.push(Box::new(ApiGatewayV2ManagementRouter::new(apigwv2_mgmt_service)));

    // API Gateway v2 execution engine (path prefix: /_aws/execute-api/)
    #[cfg(feature = "apigatewayv2")]
    services.push(Box::new(ApiGatewayV2ExecutionRouter::new(apigwv2_exec_handler)));

    #[cfg(feature = "lambda")]
    services.push(Box::new(LambdaServiceRouter::new(lambda_service)));

    // ... other services ...

    #[cfg(feature = "s3")]
    services.push(Box::new(S3ServiceRouter::new(s3_service))); // catch-all, must be last

    GatewayService::new(services)
}
```

### 12.4 Configuration

```rust
pub struct ApiGatewayV2Config {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub account_id: String,
    /// Gateway host address (used in API endpoint URL generation).
    pub host: String,
    /// Gateway port (used in API endpoint URL generation).
    pub port: u16,
}

impl ApiGatewayV2Config {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("APIGATEWAYV2_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            host: env_str("GATEWAY_HOST", "localhost"),
            port: env_u16("GATEWAY_PORT", 4566),
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared with all services) |
| `APIGATEWAYV2_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 verification |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default AWS account ID |

### 12.6 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "running",
        "dynamodb": "running",
        "sqs": "running",
        "lambda": "running",
        "apigatewayv2": "running"
    }
}
```

### 12.7 Cross-Service Dependency: Lambda

The API Gateway v2 execution engine needs to invoke Lambda functions for `AWS_PROXY` integrations. This creates a cross-service dependency:

```rust
// In rustack-apigatewayv2-core/Cargo.toml
[features]
default = ["lambda-integration"]
lambda-integration = ["dep:rustack-lambda-core"]

[dependencies]
rustack-lambda-core = { workspace = true, optional = true }
```

At server initialization, the Lambda provider is shared with the API Gateway v2 provider:

```rust
// In apps/rustack/src/main.rs
#[cfg(all(feature = "apigatewayv2", feature = "lambda"))]
{
    let apigwv2_provider = RustackApiGatewayV2::new(
        apigwv2_config,
        Some(Arc::clone(&lambda_provider)), // Share Lambda provider
        http_client,
    );
}
```

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Management router**: test URL path matching for all ~53 operations, path parameter extraction, ambiguous paths (e.g., distinguishing GetApi from GetApis)
- **Route matching**: test exact match, parameterized match, greedy match, ANY method, `$default` fallback, priority ordering
- **Path template matching**: test `{id}` extraction, `{proxy+}` greedy capture, literal segments, edge cases (empty path, root path)
- **Lambda event construction**: test payload format 2.0 event JSON structure, base64 encoding, header mapping, query string parsing
- **Lambda response parsing**: test format 2.0 response (statusCode, headers, body, cookies, isBase64Encoded), simple string response
- **CORS handling**: test preflight response generation, CORS header injection, origin matching, wildcard origins
- **Auto-deploy**: test that route/integration changes trigger deployment creation when stage has auto_deploy enabled
- **ID generation**: test format (10-char alphanumeric)
- **Route key parsing**: test "METHOD /path" format, $default, ANY, invalid formats

### 13.2 Integration Tests with aws-sdk-apigatewayv2

```rust
// tests/integration/apigatewayv2_tests.rs
#[tokio::test]
#[ignore]
async fn test_should_create_get_delete_api() {
    let client = aws_sdk_apigatewayv2::Client::new(&config);

    // Create HTTP API.
    let create = client.create_api()
        .name("test-api")
        .protocol_type(ProtocolType::Http)
        .send().await.unwrap();

    let api_id = create.api_id().unwrap();
    assert_eq!(create.name(), Some("test-api"));
    assert_eq!(create.protocol_type(), Some(&ProtocolType::Http));

    // Get API.
    let get = client.get_api()
        .api_id(api_id)
        .send().await.unwrap();
    assert_eq!(get.name(), Some("test-api"));

    // Delete API.
    client.delete_api()
        .api_id(api_id)
        .send().await.unwrap();

    // Verify deleted.
    let err = client.get_api()
        .api_id(api_id)
        .send().await;
    assert!(err.is_err());
}

#[tokio::test]
#[ignore]
async fn test_should_create_route_with_lambda_integration() {
    let apigw_client = aws_sdk_apigatewayv2::Client::new(&config);
    let lambda_client = aws_sdk_lambda::Client::new(&config);

    // Create Lambda function.
    let zip_bytes = create_test_zip("handler.py",
        b"def handler(event, ctx): return {'statusCode': 200, 'body': 'hello'}");
    lambda_client.create_function()
        .function_name("api-handler")
        .runtime(Runtime::Python312)
        .handler("handler.handler")
        .role("arn:aws:iam::000000000000:role/test-role")
        .code(FunctionCode::builder().zip_file(Blob::new(zip_bytes)).build())
        .send().await.unwrap();

    // Create HTTP API.
    let api = apigw_client.create_api()
        .name("test-api")
        .protocol_type(ProtocolType::Http)
        .send().await.unwrap();
    let api_id = api.api_id().unwrap();

    // Create Lambda integration.
    let integration = apigw_client.create_integration()
        .api_id(api_id)
        .integration_type(IntegrationType::AwsProxy)
        .integration_uri("arn:aws:lambda:us-east-1:000000000000:function:api-handler")
        .payload_format_version("2.0")
        .send().await.unwrap();
    let integration_id = integration.integration_id().unwrap();

    // Create route.
    let route = apigw_client.create_route()
        .api_id(api_id)
        .route_key("GET /hello")
        .target(format!("integrations/{integration_id}"))
        .send().await.unwrap();
    assert_eq!(route.route_key(), Some("GET /hello"));

    // Create stage.
    apigw_client.create_stage()
        .api_id(api_id)
        .stage_name("$default")
        .auto_deploy(true)
        .send().await.unwrap();

    // Invoke via execution endpoint (Phase 4).
    let resp = reqwest::get(format!(
        "http://localhost:4566/_aws/execute-api/{api_id}/$default/hello"
    )).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "hello");
}

#[tokio::test]
#[ignore]
async fn test_should_handle_cors_preflight() {
    let client = aws_sdk_apigatewayv2::Client::new(&config);

    // Create API with CORS.
    let api = client.create_api()
        .name("cors-api")
        .protocol_type(ProtocolType::Http)
        .cors_configuration(Cors::builder()
            .allow_origins("*")
            .allow_methods("GET")
            .allow_methods("POST")
            .allow_headers("content-type")
            .max_age(3600)
            .build())
        .send().await.unwrap();
    let api_id = api.api_id().unwrap();

    // Create stage.
    client.create_stage()
        .api_id(api_id)
        .stage_name("$default")
        .auto_deploy(true)
        .send().await.unwrap();

    // Send OPTIONS preflight.
    let resp = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS,
            format!("http://localhost:4566/_aws/execute-api/{api_id}/$default/anything"))
        .header("origin", "https://example.com")
        .header("access-control-request-method", "POST")
        .send().await.unwrap();

    assert_eq!(resp.status(), 204);
    assert_eq!(
        resp.headers().get("access-control-allow-origin").unwrap().to_str().unwrap(),
        "https://example.com"
    );
}
```

### 13.3 Third-Party Test Suites

#### 13.3.1 LocalStack API Gateway v2 Test Suite (Primary)

The LocalStack test suite for API Gateway v2 is located at `vendors/localstack/tests/aws/services/apigateway/`. Key test files:

| File | Coverage |
|------|---------|
| `test_apigateway_http.py` | HTTP API management + execution |
| `test_apigateway_common.py` | Shared test utilities |

**Adaptation strategy**: Run the Python test suite against Rustack's API Gateway v2 endpoint. Focus on HTTP API tests (not REST API or WebSocket).

```makefile
test-apigatewayv2-localstack:
	@cd vendors/localstack && python -m pytest tests/aws/services/apigateway/test_apigateway_http.py \
	    --endpoint-url=http://localhost:4566 -v
```

#### 13.3.2 AWS CLI Smoke Tests

```bash
ENDPOINT="--endpoint-url http://localhost:4566"

# Create HTTP API
API_ID=$(aws apigatewayv2 create-api $ENDPOINT \
    --name test-api \
    --protocol-type HTTP \
    --query 'ApiId' --output text)

echo "Created API: $API_ID"

# Create integration
INTEGRATION_ID=$(aws apigatewayv2 create-integration $ENDPOINT \
    --api-id $API_ID \
    --integration-type HTTP_PROXY \
    --integration-uri https://httpbin.org/get \
    --integration-method GET \
    --payload-format-version "1.0" \
    --query 'IntegrationId' --output text)

# Create route
aws apigatewayv2 create-route $ENDPOINT \
    --api-id $API_ID \
    --route-key "GET /test" \
    --target "integrations/$INTEGRATION_ID"

# Create stage
aws apigatewayv2 create-stage $ENDPOINT \
    --api-id $API_ID \
    --stage-name '$default' \
    --auto-deploy

# List APIs
aws apigatewayv2 get-apis $ENDPOINT

# Delete API
aws apigatewayv2 delete-api $ENDPOINT --api-id $API_ID
```

### 13.4 Makefile Targets

```makefile
test-apigatewayv2: test-apigatewayv2-unit test-apigatewayv2-integration

test-apigatewayv2-unit:
	@cargo test -p rustack-apigatewayv2-model -p rustack-apigatewayv2-core -p rustack-apigatewayv2-http

test-apigatewayv2-integration:
	@cargo test -p integration-tests -- apigatewayv2 --ignored

test-apigatewayv2-cli:
	@./tests/apigatewayv2-cli-smoke.sh
```

---

## 14. Phased Implementation Plan

### Phase 0: Core API Management (15 Operations)

**Goal**: CRUD for APIs, routes, and integrations. This covers the minimum management API needed for tools like SAM, CDK, and Terraform to create HTTP API configurations.
**Estimated scope**: ~4,000-5,000 lines of Rust code across 3 new crates.

#### Step 0.1: Codegen for API Gateway v2

- Add `apigatewayv2.toml` to `codegen/services/`
- Download API Gateway v2 Smithy model JSON from `aws-models`
- Generate `rustack-apigatewayv2-model` crate
- Verify generated types compile and serde works correctly

#### Step 0.2: HTTP Layer

- Implement management API router (reuse restJson1 from Lambda)
- Implement request deserialization for all Phase 0 operations
- Implement response serialization with camelCase JSON
- Implement error formatting with `X-Amzn-Errortype` header

#### Step 0.3: Storage Engine

- Implement `ApiStore` with `DashMap<String, ApiRecord>`
- Implement `ApiRecord`, `RouteRecord`, `IntegrationRecord`
- Implement ID generation (10-char alphanumeric)
- Implement ARN construction

#### Step 0.4: Core Operations (15 ops)

- `CreateApi`, `GetApi`, `UpdateApi`, `DeleteApi`, `GetApis`
- `CreateRoute`, `GetRoute`, `UpdateRoute`, `DeleteRoute`, `GetRoutes`
- `CreateIntegration`, `GetIntegration`, `UpdateIntegration`, `DeleteIntegration`, `GetIntegrations`

#### Step 0.5: Server Integration

- Implement `ApiGatewayV2ManagementRouter` with `/v2/` path prefix matching
- Add `apigatewayv2` cargo feature gate
- Register in gateway before S3
- Update health endpoint

#### Step 0.6: Testing

- Unit tests for router, path matching, CRUD operations
- Integration tests with `aws-sdk-apigatewayv2`
- CLI smoke tests

### Phase 1: Stages, Deployments, Route Responses (12 Operations)

**Goal**: Stage management, deployment snapshots, auto-deploy, route responses.

- `CreateStage`, `GetStage`, `UpdateStage`, `DeleteStage`, `GetStages`
- `CreateDeployment`, `GetDeployment`, `DeleteDeployment`, `GetDeployments`
- `CreateRouteResponse`, `GetRouteResponse`, `DeleteRouteResponse`, `GetRouteResponses`
- Auto-deploy logic: when a stage has `auto_deploy: true`, changes to routes/integrations trigger automatic deployment creation
- `$default` stage support

### Phase 2: Authorizers and Models (11 Operations)

**Goal**: JWT authorizer configuration, model schema management.

- `CreateAuthorizer`, `GetAuthorizer`, `UpdateAuthorizer`, `DeleteAuthorizer`, `GetAuthorizers`
- `CreateModel`, `GetModel`, `UpdateModel`, `DeleteModel`, `GetModels`, `GetModelTemplate`
- JWT authorizer validation (issuer, audience) in the execution engine

### Phase 3: Domain Names, VPC Links, Tags, API Mappings (18 Operations)

**Goal**: Complete the management API surface.

- `CreateDomainName`, `GetDomainName`, `UpdateDomainName`, `DeleteDomainName`, `GetDomainNames`
- `CreateVpcLink`, `GetVpcLink`, `UpdateVpcLink`, `DeleteVpcLink`, `GetVpcLinks`
- `TagResource`, `UntagResource`, `GetTags`
- `CreateApiMapping`, `GetApiMapping`, `UpdateApiMapping`, `DeleteApiMapping`, `GetApiMappings`

### Phase 4: Execution Engine

**Goal**: Actually execute HTTP API requests through configured routes/integrations.
**Estimated scope**: ~3,000-4,000 lines for the execution engine.

#### Step 4.1: Execution Router

- Implement `ApiGatewayV2ExecutionRouter` with `/_aws/execute-api/` path prefix matching
- Parse execution target (api_id, stage, proxy path) from URL
- Register execution router in gateway

#### Step 4.2: Route Matching

- Implement path template matching with `{param}` extraction
- Implement greedy path parameters `{proxy+}`
- Implement `$default` catch-all route
- Implement `ANY` method matching
- Implement match priority ordering

#### Step 4.3: Lambda Proxy Integration

- Build API Gateway v2 payload format 2.0 event
- Invoke Lambda function via `rustack-lambda-core`
- Parse Lambda response (format 2.0, simple string)
- Handle Lambda errors (function error, timeout)

#### Step 4.4: HTTP Proxy Integration

- Forward requests to backend HTTP URLs via `reqwest`
- Substitute path parameters and stage variables in URI
- Forward headers and body
- Handle backend errors and timeouts

#### Step 4.5: Mock Integration

- Return configured static responses
- Support request/response templates (basic)

#### Step 4.6: CORS Handling

- Implement automatic CORS preflight responses
- Add CORS headers to all responses when cors_configuration is present
- Origin matching (exact and wildcard)

#### Step 4.7: Integration Testing

- End-to-end tests: create API + route + Lambda integration + stage, then invoke via execution endpoint
- HTTP proxy integration tests
- CORS tests
- Route matching tests with real HTTP requests
- Error handling tests (missing route, missing integration, Lambda error)

---

## 15. Risk Analysis

### 15.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Route matching edge cases | High | Medium | Comprehensive unit tests for path template matching. Follow AWS documentation precisely. Test with real AWS SDK to compare behavior. |
| Lambda proxy event format compatibility | High | High | Test with multiple Lambda runtimes (Python, Node.js). Validate event JSON against AWS documentation. Use LocalStack test suite as reference. |
| CORS interaction complexity | Medium | Medium | CORS is well-documented for HTTP APIs. Test with real browsers. Handle edge cases (wildcard vs specific origins, credentials). |
| PATCH semantics for updates | Medium | Low | API Gateway v2 uses PATCH (partial update) not PUT (full replacement). Implement merge semantics: only update fields present in the request body. Use `Option<T>` for all update input fields. |
| restJson1 codegen deep nesting | Medium | Medium | API Gateway v2 has deeper URL paths than Lambda (e.g., `/v2/apis/{apiId}/routes/{routeId}/routeresponses/{routeResponseId}`). The router must handle up to 7 path segments with multiple parameters. |
| Cross-service Lambda dependency | Medium | High | Gate behind `lambda-integration` feature. When Lambda is not available, return a clear error for `AWS_PROXY` integrations. Allow API management to work independently. |
| Auto-deploy race conditions | Low | Medium | Use DashMap entry API for atomic updates. Auto-deploy creates a new deployment only if the stage still has auto_deploy enabled after the route/integration change. |
| Execution engine URL conflicts | Low | High | `/_aws/execute-api/` prefix is distinctive and cannot conflict with S3, Lambda, or other service paths. Matches LocalStack's convention. |
| HTTP proxy timeout handling | Medium | Medium | Use configurable timeout (integration `timeout_in_millis` field). Default 30 seconds. Return 504 on timeout. |

### 15.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users expect WebSocket support | Medium | Medium | Clearly document as non-goal. Accept `protocol_type: WEBSOCKET` in CreateApi but return error on execution. Design storage to allow adding WebSocket later. |
| Users expect request/response transformations | High | Medium | Accept and store mapping templates but do not evaluate them. Document limitation. Most users with Lambda proxy integration do not need transformations (payload format 2.0 handles the mapping). |
| Users expect API Gateway v1 (REST APIs) | Medium | Medium | Clearly document that only v2 (HTTP APIs) is supported. Most modern tooling defaults to v2. |
| Terraform creates both v1 and v2 resources | Medium | Medium | Terraform uses different resource types (`aws_apigatewayv2_*` vs `aws_api_gateway_*`). Only v2 resources will work. |
| SAM CLI expects specific API endpoint format | High | High | Test early with SAM CLI. SAM constructs execute-api URLs from the API ID and stage. Ensure `/_aws/execute-api/{apiId}/{stage}/{path}` format matches what SAM generates. |

### 15.3 Behavioral Differences

| Behavior | AWS API Gateway v2 | LocalStack | Rustack | Justification |
|----------|-------------------|------------|-----------|---------------|
| API endpoint format | `https://{apiId}.execute-api.{region}.amazonaws.com` | `http://localhost:4566/_aws/execute-api/{apiId}` | `http://localhost:4566/_aws/execute-api/{apiId}` | Follow LocalStack convention |
| Default stage | Auto-created `$default` stage | Configurable | Explicit creation required | Simpler, more predictable |
| Deployment validation | Validates routes have integrations | Minimal validation | Minimal validation | Simpler for local dev |
| JWT authorizer | Full OIDC validation | Basic validation | Store config, basic validation | Full OIDC requires key fetching |
| Lambda authorizer | Invokes Lambda function | Invokes Lambda | Not supported (stores config) | Avoid circular complexity |
| Throttling | Enforced per-route/stage | Not enforced | Not enforced | Not needed for local dev |
| Access logging | CloudWatch Logs | Local file | Not implemented | Avoid CloudWatch dependency |
| Custom domains | DNS + TLS provisioning | Simulated | Metadata only | No DNS/TLS in local env |
| VPC links | Actual VPC connectivity | Simulated | Metadata only | No VPC in local env |
| Mutual TLS | Certificate validation | Simulated | Not implemented | Complex PKI not needed locally |

---

## Appendix A: API Gateway v2 vs Other Services Implementation Effort Comparison

| Component | Lambda Lines | Secrets Manager Lines | APIGW v2 Est. | Notes |
|-----------|-------------|---------------------|----------------|-------|
| Model (codegen output) | ~3,500 | ~2,500 | ~4,500 | ~53 operations, many shared types |
| HTTP routing | ~600 | ~100 | ~700 | restJson1 reuse, deeper paths |
| Request/response codec | ~800 | ~200 | ~800 | restJson1 reuse |
| Auth integration | ~100 | ~100 | ~100 | SigV4 only, identical |
| Core business logic | ~3,000 | ~2,000 | ~3,500 | CRUD is straightforward, execution adds complexity |
| Storage engine | ~1,500 | ~1,000 | ~1,200 | DashMap with nested HashMaps |
| Execution engine | ~2,500 (Docker) | N/A | ~3,500 | Route matching + Lambda/HTTP proxy + CORS |
| **Total** | **~12,500** | **~5,800** | **~14,300** | |

## Appendix B: API Gateway v2 Error Codes

| Error Code | HTTP Status | When |
|-----------|------------|------|
| `NotFoundException` | 404 | API, route, integration, stage, etc. not found |
| `ConflictException` | 409 | Duplicate route key, resource already exists |
| `BadRequestException` | 400 | Invalid input (bad route key format, missing required field) |
| `TooManyRequestsException` | 429 | API rate limit exceeded |
| `AccessDeniedException` | 403 | Insufficient permissions |
| `InternalServerErrorException` | 500 | Internal server error |

## Appendix C: API Gateway v2 Execution Endpoint URL Patterns

| Pattern | Example | Description |
|---------|---------|-------------|
| Management | `POST http://localhost:4566/v2/apis` | Create API |
| Management | `GET http://localhost:4566/v2/apis/{apiId}/routes` | List routes |
| Execution | `GET http://localhost:4566/_aws/execute-api/{apiId}/{stage}/items` | Execute API |
| Execution | `POST http://localhost:4566/_aws/execute-api/{apiId}/{stage}/items` | Execute API |
| Execution | `GET http://localhost:4566/_aws/execute-api/{apiId}/{stage}/items/42` | Execute with path param |

## Appendix D: Lambda Proxy Integration Payload Format 2.0

```json
{
  "version": "2.0",
  "routeKey": "GET /items/{id}",
  "rawPath": "/items/42",
  "rawQueryString": "filter=active",
  "headers": {
    "content-type": "application/json",
    "user-agent": "curl/7.68.0"
  },
  "queryStringParameters": {
    "filter": "active"
  },
  "pathParameters": {
    "id": "42"
  },
  "requestContext": {
    "accountId": "000000000000",
    "apiId": "abc1234def",
    "domainName": "abc1234def.execute-api.us-east-1.amazonaws.com",
    "domainPrefix": "abc1234def",
    "http": {
      "method": "GET",
      "path": "/items/42",
      "protocol": "HTTP/1.1",
      "sourceIp": "127.0.0.1",
      "userAgent": "curl/7.68.0"
    },
    "requestId": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "routeKey": "GET /items/{id}",
    "stage": "$default",
    "time": "19/Mar/2026:10:30:00 +0000",
    "timeEpoch": 1774044600000
  },
  "body": null,
  "isBase64Encoded": false
}
```

## Appendix E: Lambda Response Format 2.0

```json
{
  "statusCode": 200,
  "headers": {
    "content-type": "application/json",
    "x-custom-header": "value"
  },
  "cookies": [
    "session=abc123; HttpOnly; Secure"
  ],
  "body": "{\"id\": 42, \"name\": \"item\"}",
  "isBase64Encoded": false
}
```
