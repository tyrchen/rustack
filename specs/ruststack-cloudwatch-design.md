# Rustack CloudWatch Metrics & Alarms: Native Rust Implementation Design

**Date:** 2026-03-19
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-sns-design.md](./rustack-sns-design.md), [rustack-logs-design.md](./rustack-logs-design.md)
**Scope:** Add CloudWatch Metrics & Alarms support to Rustack -- ~31 operations across three phases covering metric ingestion, retrieval, aggregation, alarms, dashboards, anomaly detection, and metric streams. Uses the `awsQuery` protocol (XML responses), the same protocol infrastructure established by SNS.

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

This spec proposes adding CloudWatch Metrics & Alarms support to Rustack. Key points:

- **Completes the observability story** -- Rustack already implements CloudWatch Logs (`rustack-logs-{model,http,core}` using `awsJson1.1`). CloudWatch Metrics is the natural companion, covering metric ingestion, statistical queries, alarms, and dashboards. Together they provide the full CloudWatch experience for local development.
- **Moderate scope** -- ~31 operations across three phases. Phase 0 delivers ~15 core operations (metrics CRUD, alarms, tags) sufficient for most local development workflows. Phase 1 adds composite alarms and dashboards (~8 ops). Phase 2 adds anomaly detection and metric streams (~8 ops).
- **Different service, different protocol** -- despite sharing the "CloudWatch" brand name, CloudWatch Metrics is a completely separate AWS service from CloudWatch Logs. It uses the `awsQuery` protocol (form-urlencoded requests, XML responses) with SigV4 service name `monitoring`, whereas CloudWatch Logs uses `awsJson1.1` with service name `logs`. They share no API operations, no data structures, and no wire format.
- **awsQuery protocol reuse from SNS** -- CloudWatch Metrics uses the same `awsQuery` protocol as SNS. The entire form-encoded request parsing, XML response serialization, and error formatting infrastructure from `rustack-sns-http` can be reused. The key difference is the XML namespace (`http://monitoring.amazonaws.com/doc/2010-08-01/`) and the Action names.
- **Time-series storage engine** -- the core data structure is a time-series store keyed by `(namespace, metric_name, sorted_dimensions)`. Each series stores a `Vec<DataPoint>` with timestamp, value, unit, and optional `StatisticValues`/`Values`/`Counts` for pre-aggregated data. Range queries and period-aligned aggregation support `GetMetricStatistics` and `GetMetricData`.
- **Alarm evaluation** -- for MVP, alarms are stored and their state is managed via explicit `SetAlarmState` calls. Future phases can add a background alarm evaluator that periodically checks metric values against thresholds and triggers alarm actions (SNS publish, Lambda invoke).
- **Smithy codegen reuse** -- generate a `rustack-cloudwatch-model` crate from the official Smithy model (`monitoring-2010-08-01.json`) using the same codegen infrastructure as all other services.
- **Estimated effort** -- 4-5 days for Phase 0 (15 core operations), 2-3 days for Phase 1 (composite alarms + dashboards), 2-3 days for Phase 2 (anomaly detection + streams), plus 1 day for CI integration. Total: ~10-12 days.

---

## 2. Motivation

### 2.1 Why CloudWatch Metrics?

CloudWatch Metrics is the universal metrics destination for AWS workloads. With CloudWatch Logs already implemented, CloudWatch Metrics is the natural next step to complete Rustack's observability surface:

- **Custom application metrics** -- every non-trivial AWS application publishes custom metrics via `PutMetricData`. Without a local endpoint, developers either skip metrics testing or make real AWS API calls during development.
- **Infrastructure metrics** -- ECS, EKS, Lambda, and other services emit metrics to CloudWatch. Local emulations of these services (already in Rustack) benefit from having a metrics endpoint to validate metric publication logic.
- **Alarms and alerting** -- `PutMetricAlarm` is one of the most commonly used CloudWatch operations in IaC. Terraform `aws_cloudwatch_metric_alarm` and CDK `Alarm` constructs need a working endpoint for `plan`/`synth` cycles.
- **Prometheus remote write adapter** -- the CloudWatch Prometheus remote write adapter (`aws-otel-collector` with `awsprometheusremotewrite` exporter) uses `PutMetricData` to send Prometheus metrics to CloudWatch. A local endpoint enables testing observability pipelines.
- **Datadog, New Relic, Grafana Cloud agents** -- monitoring agents that read from CloudWatch via `GetMetricData`/`GetMetricStatistics` can be tested locally.
- **Dashboards** -- `PutDashboard`/`GetDashboard` are used by IaC tools and are trivial to implement (JSON blob storage).
- **EventBridge integration** -- alarm state changes emit EventBridge events. With EventBridge already implemented in Rustack, this enables testing complete alarm-to-action pipelines.

### 2.2 Relationship to CloudWatch Logs

CloudWatch Metrics and CloudWatch Logs are **completely separate AWS services** that happen to share a brand name:

| Aspect | CloudWatch Logs | CloudWatch Metrics |
|--------|----------------|-------------------|
| AWS service name | `logs` | `monitoring` |
| Protocol | `awsJson1.1` | `awsQuery` |
| Request format | JSON body | form-urlencoded body |
| Response format | JSON | XML |
| Operation dispatch | `X-Amz-Target: Logs_20140328.*` | `Action=PutMetricData` form param |
| SigV4 signing service | `logs` | `monitoring` |
| Smithy namespace | `com.amazonaws.cloudwatchlogs` | `com.amazonaws.cloudwatch` |
| Rustack crate prefix | `rustack-logs-*` | `rustack-cloudwatch-*` |

They share no code, no storage, no types, and no protocol infrastructure. The only connection is that CloudWatch Logs metric filters can publish metrics to CloudWatch Metrics -- a cross-service integration that is a non-goal for MVP.

### 2.3 Complexity Assessment

| Dimension | CloudWatch Metrics | SNS | CloudWatch Logs | SSM |
|-----------|-------------------|-----|-----------------|-----|
| Total operations | ~31 | ~41 | ~35 | 13 |
| Protocol | awsQuery (reuse SNS) | awsQuery | awsJson1.1 | awsJson1.1 |
| Storage complexity | Time-series + alarm state + dashboards | Topics + subscriptions | Log streams + events | Parameters + versions |
| Background processing | Alarm evaluation (optional) | Message delivery | None (MVP) | None |
| Cross-service deps | SNS (alarm actions), Lambda (alarm actions) | SQS (fan-out) | None | None |
| Concurrency model | Request/response + optional evaluator | Request/response + delivery | Request/response | Request/response |
| Estimated lines of code | ~5,000 | ~6,000 | ~4,500 | ~3,000 |

CloudWatch Metrics is moderately complex. The statistical aggregation engine (computing Sum, Average, Min, Max, SampleCount, and percentiles over time-series data) is the core algorithmic challenge. Alarm evaluation is conceptually simple but requires cross-service integration for actions.

### 2.4 Tool Coverage

With Phase 0 implemented (~15 operations), the following tools work out of the box:

| Tool | Operations Used | Phase Available |
|------|----------------|-----------------|
| AWS CLI (`aws cloudwatch`) | All metric/alarm CRUD ops | Phase 0 |
| Terraform (`aws_cloudwatch_metric_alarm`) | PutMetricAlarm, DescribeAlarms, DeleteAlarms, tags | Phase 0 |
| Terraform (`aws_cloudwatch_dashboard`) | PutDashboard, GetDashboard, DeleteDashboards | Phase 1 |
| AWS CDK (`Alarm`, `Metric`, `Dashboard`) | PutMetricAlarm, PutMetricData, PutDashboard | Phase 0 + Phase 1 |
| Prometheus remote write adapter | PutMetricData | Phase 0 |
| Datadog Agent (CloudWatch integration) | GetMetricData, ListMetrics | Phase 0 |
| Grafana CloudWatch data source | GetMetricData, GetMetricStatistics, ListMetrics | Phase 0 |
| Custom application metrics | PutMetricData, GetMetricStatistics | Phase 0 |
| CloudFormation/SAM | PutMetricAlarm, PutDashboard | Phase 0 + Phase 1 |
| Pulumi | PutMetricAlarm, PutMetricData, PutDashboard | Phase 0 + Phase 1 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Full core metrics API** -- implement PutMetricData, GetMetricData, GetMetricStatistics, ListMetrics for metric ingestion and retrieval
2. **Alarm management** -- PutMetricAlarm, DescribeAlarms, DescribeAlarmsForMetric, DeleteAlarms, SetAlarmState, EnableAlarmActions, DisableAlarmActions, DescribeAlarmHistory
3. **Tag support** -- TagResource, UntagResource, ListTagsForResource for both alarms and rules
4. **Statistical aggregation** -- compute Sum, Average, Minimum, Maximum, SampleCount over configurable time periods
5. **Dimension-based metric identification** -- metrics uniquely identified by (namespace, metric_name, dimensions) with normalized dimension ordering
6. **Dashboard storage** -- PutDashboard, GetDashboard, DeleteDashboards, ListDashboards (JSON blob storage)
7. **Composite alarms** -- PutCompositeAlarm (store configuration, evaluate on explicit state change)
8. **awsQuery protocol** -- full form-urlencoded request parsing and XML response serialization, reusing SNS protocol infrastructure
9. **Smithy-generated types** -- all types generated from official AWS Smithy model
10. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the awsQuery protocol layer from SNS
11. **Same Docker image** -- single binary serves all 12 services on port 4566
12. **Pass LocalStack CloudWatch test suite** -- validate against vendored tests

### 3.2 Non-Goals

1. **Automatic alarm evaluation** -- for MVP, alarm state changes only via explicit `SetAlarmState` calls. No background periodic evaluation against metric data. Future phases can add a background evaluator.
2. **Alarm action execution** -- alarm actions (SNS publish, Lambda invoke, Auto Scaling) are stored but not executed for MVP. Future: integrate with SNS and Lambda services when alarm state changes.
3. **Math expressions in GetMetricData** -- `MetricDataQueries` with `Expression` field (e.g., `METRICS("m1") / METRICS("m2")`) are a non-goal. Only simple `MetricStat` queries are supported.
4. **Extended statistics (percentiles)** -- pNN percentile computation (e.g., p99, p95) deferred to future phases. Only basic statistics (Sum, Average, Min, Max, SampleCount) in MVP.
5. **Anomaly detection model training** -- `PutAnomalyDetector` stores configuration but does not train or evaluate anomaly detection models
6. **Metric streams to Firehose** -- `PutMetricStream` stores configuration but does not actually stream metrics
7. **Insight rules evaluation** -- `PutInsightRule` stores rules but does not evaluate them against log/metric data
8. **Cross-account/cross-region metric aggregation** -- single-account, single-region for local development
9. **High-resolution metrics** -- accept and store sub-minute periods but no special high-resolution storage optimization
10. **CloudWatch Logs metric filter integration** -- metric filters in CloudWatch Logs do not publish to CloudWatch Metrics in MVP
11. **Data persistence across restarts** -- in-memory only, matching all other Rustack services
12. **Real CloudWatch Contributor Insights** -- insight rules accepted but not evaluated
13. **Metric data retention limits** -- real AWS retains data for 15 months with resolution-based tiers; for local dev, keep all data in memory with a configurable maximum retention period

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                AWS SDK / CLI / Terraform / Prometheus adapter / Grafana
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  Routes by X-Amz-Target, Content-Type, Action=
              |   (ServiceRouter)   |
              +--------+------------+
                       |
         +------+------+------+------+------+------+
         v      v      v      v      v      v      v
   +------+ +------+ +------+ +------+ +------+ +------+ +--------+
   | S3   | | DDB  | | SQS  | | SSM  | | SNS  | | Logs | | CWMet  |
   | HTTP | | HTTP | | HTTP | | HTTP | | HTTP | | HTTP | | HTTP   |
   +------+ +------+ +------+ +------+ +------+ +------+ +--------+
      |        |        |       |        |        |          |
   +------+ +------+ +------+ +------+ +------+ +------+ +--------+
   | S3   | | DDB  | | SQS  | | SSM  | | SNS  | | Logs | | CWMet  |
   | Core | | Core | | Core | | Core | | Core | | Core | | Core   |
   +------+ +------+ +------+ +------+ +------+ +------+ +--------+
      |        |        |       |        |        |          |
      +--------+--------+-------+--------+--------+----------+
                                |
                         +------+------+
                         | rustack-  |
                         | core + auth |
                         +-------------+
```

### 4.2 Relationship with CloudWatch Logs

CloudWatch Metrics and CloudWatch Logs are completely independent services within Rustack:

- **Separate crates**: `rustack-cloudwatch-{model,core,http}` vs `rustack-logs-{model,core,http}`
- **Separate protocols**: awsQuery (XML) vs awsJson1.1 (JSON)
- **Separate routing**: Action= form parameter vs X-Amz-Target header
- **Separate storage**: MetricStore/AlarmStore vs LogGroupStore/LogStreamStore
- **No shared code** beyond `rustack-core` and `rustack-auth`

The naming convention uses `cloudwatch` (not `monitoring`) for the crate prefix to match the common AWS branding, while the SigV4 service name is `monitoring` as required by the AWS API.

### 4.3 Gateway Routing

CloudWatch Metrics uses `awsQuery` with `Content-Type: application/x-www-form-urlencoded` -- the same wire format as SNS. The gateway must distinguish between them.

**Current awsQuery routing**: SNS currently matches all `POST` requests with `Content-Type: application/x-www-form-urlencoded`. With CloudWatch Metrics also using awsQuery, we need to refine this. There are two approaches:

**Approach A: SigV4 Authorization header parsing** -- The `Authorization` header contains the SigV4 credential scope which includes the service name: `Credential=AKID/20260319/us-east-1/monitoring/aws4_request`. Parsing the service name from the credential scope unambiguously identifies the target service:
- `monitoring` -> CloudWatch Metrics
- `sns` -> SNS

**Approach B: Action-based routing** -- Buffer the form body and inspect the `Action=` parameter. CloudWatch Metrics actions (`PutMetricData`, `DescribeAlarms`, etc.) are distinct from SNS actions (`Publish`, `CreateTopic`, etc.). This is already the documented approach in the SNS design spec for distinguishing SNS from SQS awsQuery.

**Recommended: Approach A (SigV4 credential parsing)** with Approach B as fallback. SigV4 credential parsing is:
- Available before reading the body (header-only)
- Unambiguous (service name is explicit)
- Forward-compatible (any future awsQuery service is automatically distinguishable)
- Already available in the `Authorization` header that `rustack-auth` processes

For the SNS router, the existing `content-type` match continues to work because CloudWatch Metrics will be checked first (via SigV4 service name). The routing order becomes:

1. Check `X-Amz-Target` headers (DynamoDB, SQS, SSM, Logs, KMS, Kinesis, SecretsManager, EventBridge)
2. Check Lambda path patterns
3. For `POST /` with `Content-Type: application/x-www-form-urlencoded`:
   a. Parse SigV4 `Authorization` header for service name
   b. If `monitoring` -> CloudWatch Metrics
   c. If `sns` -> SNS
   d. Fallback: parse `Action=` parameter from body to disambiguate
4. Default: S3 (catch-all)

```rust
/// Extract the SigV4 signing service name from the Authorization header.
///
/// The Authorization header format is:
/// `AWS4-HMAC-SHA256 Credential=AKID/date/region/service/aws4_request, ...`
///
/// Returns the service name (e.g., "monitoring", "sns", "s3").
fn extract_sigv4_service(req: &http::Request<Incoming>) -> Option<&str> {
    let auth = req.headers().get("authorization")?.to_str().ok()?;
    // Find Credential= and extract the service component
    let cred_start = auth.find("Credential=")? + "Credential=".len();
    let cred_end = auth[cred_start..].find(',').map(|i| cred_start + i)
        .unwrap_or(auth.len());
    let credential = &auth[cred_start..cred_end];
    // Format: AKID/date/region/service/aws4_request
    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() >= 4 {
        Some(parts[3])
    } else {
        None
    }
}
```

### 4.4 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-sns-{model,core,http}
+-- rustack-events-{model,core,http}
+-- rustack-logs-{model,core,http}
+-- rustack-kms-{model,core,http}
+-- rustack-kinesis-{model,core,http}
+-- rustack-secretsmanager-{model,core,http}
+-- rustack-cloudwatch-model        <-- NEW (auto-generated)
+-- rustack-cloudwatch-core         <-- NEW
+-- rustack-cloudwatch-http         <-- NEW

rustack-cloudwatch-http
+-- rustack-cloudwatch-model
+-- rustack-auth
+-- quick-xml (XML response serialization, reuse from SNS)
+-- serde_urlencoded (form request deserialization, reuse from SNS)

rustack-cloudwatch-core
+-- rustack-core
+-- rustack-cloudwatch-model
+-- dashmap
+-- tokio
+-- serde_json (for dashboard body storage)

rustack-cloudwatch-model (auto-generated, standalone)
+-- serde
+-- serde_json
```

---

## 5. Protocol Design: awsQuery

### 5.1 Protocol Overview

CloudWatch Metrics uses the `@awsQuery` protocol, identical in wire format to SNS. Requests are `POST /` with `Content-Type: application/x-www-form-urlencoded`. Responses are XML. The API version is `2010-08-01`.

| Aspect | CloudWatch Metrics (awsQuery) | SNS (awsQuery) |
|--------|------------------------------|----------------|
| Content-Type (request) | `application/x-www-form-urlencoded` | `application/x-www-form-urlencoded` |
| Content-Type (response) | `text/xml` | `text/xml` |
| HTTP Method | POST | POST |
| URL Path | `/` | `/` |
| Operation dispatch | `Action=<OperationName>` form parameter | `Action=<OperationName>` form parameter |
| API version | `2010-08-01` | `2010-03-31` |
| XML namespace | `http://monitoring.amazonaws.com/doc/2010-08-01/` | `http://sns.amazonaws.com/doc/2010-03-31/` |
| SigV4 service | `monitoring` | `sns` |

### 5.2 What We Reuse from SNS

The SNS implementation provides the complete awsQuery protocol infrastructure:

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| Form-urlencoded request parsing | Yes | `serde_urlencoded` for flat params; custom parser for nested `.member.N` lists |
| XML response serialization | Yes | `quick-xml` with the same `<ActionResponse>/<ActionResult>/<ResponseMetadata>` envelope |
| XML error formatting | Yes | Same `<ErrorResponse><Error><Type><Code><Message>` format |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| Multi-account/region state | Yes | `rustack-core` unchanged |

The protocol layer for CloudWatch Metrics is structurally identical to SNS. The only differences are the XML namespace, action names, and the specific form parameter structures for each operation.

### 5.3 Wire Format: PutMetricData Request

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/monitoring/aws4_request, ...

Action=PutMetricData
&Namespace=MyApp
&MetricData.member.1.MetricName=RequestCount
&MetricData.member.1.Value=42
&MetricData.member.1.Unit=Count
&MetricData.member.1.Timestamp=2026-03-19T12%3A00%3A00Z
&MetricData.member.1.Dimensions.member.1.Name=Environment
&MetricData.member.1.Dimensions.member.1.Value=Production
&MetricData.member.1.Dimensions.member.2.Name=Service
&MetricData.member.1.Dimensions.member.2.Value=API
&Version=2010-08-01
```

### 5.4 Wire Format: PutMetricData Response

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<PutMetricDataResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <ResponseMetadata>
    <RequestId>e4034a54-1f2d-4567-89ab-cdef01234567</RequestId>
  </ResponseMetadata>
</PutMetricDataResponse>
```

### 5.5 Wire Format: GetMetricStatistics Request

```http
POST / HTTP/1.1
Content-Type: application/x-www-form-urlencoded
Authorization: AWS4-HMAC-SHA256 Credential=AKID/20260319/us-east-1/monitoring/aws4_request, ...

Action=GetMetricStatistics
&Namespace=MyApp
&MetricName=RequestCount
&StartTime=2026-03-19T11%3A00%3A00Z
&EndTime=2026-03-19T12%3A00%3A00Z
&Period=300
&Statistics.member.1=Sum
&Statistics.member.2=Average
&Dimensions.member.1.Name=Environment
&Dimensions.member.1.Value=Production
&Version=2010-08-01
```

### 5.6 Wire Format: GetMetricStatistics Response

```http
HTTP/1.1 200 OK
Content-Type: text/xml

<GetMetricStatisticsResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <GetMetricStatisticsResult>
    <Label>RequestCount</Label>
    <Datapoints>
      <member>
        <Timestamp>2026-03-19T11:00:00Z</Timestamp>
        <Sum>420.0</Sum>
        <Average>42.0</Average>
        <SampleCount>10.0</SampleCount>
        <Unit>Count</Unit>
      </member>
      <member>
        <Timestamp>2026-03-19T11:05:00Z</Timestamp>
        <Sum>350.0</Sum>
        <Average>35.0</Average>
        <SampleCount>10.0</SampleCount>
        <Unit>Count</Unit>
      </member>
    </Datapoints>
  </GetMetricStatisticsResult>
  <ResponseMetadata>
    <RequestId>a1b2c3d4-5678-90ab-cdef-EXAMPLE11111</RequestId>
  </ResponseMetadata>
</GetMetricStatisticsResponse>
```

### 5.7 Wire Format: Error Response

```http
HTTP/1.1 400 Bad Request
Content-Type: text/xml

<ErrorResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <Error>
    <Type>Sender</Type>
    <Code>ResourceNotFound</Code>
    <Message>The alarm 'my-alarm' does not exist.</Message>
  </Error>
  <RequestId>d74b8436-ae13-5ab4-a9ff-ce54dfea72a0</RequestId>
</ErrorResponse>
```

### 5.8 Form Parameter Encoding Conventions

CloudWatch uses the same `awsQuery` encoding conventions as SNS:

**Flat parameters:**
```
Action=PutMetricAlarm&AlarmName=HighCPU&Namespace=AWS/EC2&MetricName=CPUUtilization
&ComparisonOperator=GreaterThanThreshold&Threshold=80&EvaluationPeriods=3&Period=300
&Statistic=Average
```

**List parameters (`.member.N`):**
```
Statistics.member.1=Sum&Statistics.member.2=Average
AlarmActions.member.1=arn:aws:sns:us-east-1:000000000000:my-topic
```

**Nested structure lists (`.member.N.Field`):**
```
MetricData.member.1.MetricName=CPUUtilization
&MetricData.member.1.Value=80.5
&MetricData.member.1.Dimensions.member.1.Name=InstanceId
&MetricData.member.1.Dimensions.member.1.Value=i-1234567890abcdef0
```

**Tags:**
```
Tags.member.1.Key=Environment&Tags.member.1.Value=Production
&Tags.member.2.Key=Team&Tags.member.2.Value=Platform
```

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-cloudwatch-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/monitoring.json` (namespace `com.amazonaws.cloudwatch`)
**Service config:** `codegen/services/cloudwatch.toml`
**Generate:** `make codegen-cloudwatch`

### 6.2 Service Configuration (TOML)

```toml
[service]
name = "cloudwatch"
display_name = "CloudWatch"
rust_prefix = "CloudWatch"
namespace = "com.amazonaws.cloudwatch"
protocol = "awsQuery"

[protocol]
serde_rename = "PascalCase"
emit_serde_derives = true

[operations]
phase0 = [
    # Metric operations
    "PutMetricData", "GetMetricData", "GetMetricStatistics", "ListMetrics",
    # Alarm operations
    "PutMetricAlarm", "DescribeAlarms", "DescribeAlarmsForMetric",
    "DeleteAlarms", "SetAlarmState",
    "EnableAlarmActions", "DisableAlarmActions",
    "DescribeAlarmHistory",
    # Tagging
    "TagResource", "UntagResource", "ListTagsForResource",
]
phase1 = [
    # Composite alarms
    "PutCompositeAlarm",
    # Dashboards
    "PutDashboard", "GetDashboard", "DeleteDashboards", "ListDashboards",
    # Insight rules
    "PutInsightRule", "DeleteInsightRules", "DescribeInsightRules",
]
phase2 = [
    # Anomaly detection
    "PutAnomalyDetector", "DescribeAnomalyDetectors",
    "DeleteAnomalyDetector",
    # Managed insight rules
    "PutManagedInsightRules",
    # Metric streams
    "PutMetricStream", "DeleteMetricStream",
    "ListMetricStreams", "GetMetricStream",
]

[errors.custom]
MissingAction = { status = 400, message = "Missing required parameter: Action" }
InvalidAction = { status = 400, message = "Operation is not supported" }

[output]
file_layout = "flat"
```

### 6.3 Generated Output

The codegen produces 6 files in `crates/rustack-cloudwatch-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs): `Dimension`, `MetricDatum`, `StatisticSet`, `Datapoint`, `AlarmType`, `ComparisonOperator`, `StateValue`, `Statistic`, `StandardUnit`, etc. |
| `operations.rs` | `CloudWatchOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `CloudWatchErrorCode` enum + `CloudWatchError` struct + `cloudwatch_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.4 Service-Specific Notes

CloudWatch uses `PascalCase` for form parameter names, matching the Smithy model field names. The codegen produces serde-compatible structs, but the HTTP layer uses custom form-encoded deserialization (not `serde_urlencoded` directly) because of the nested `.member.N` list encoding.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-cloudwatch-model` (auto-generated)

```
crates/rustack-cloudwatch-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: CloudWatchOperation enum
    +-- error.rs            # Auto-generated: error types + error codes
    +-- input.rs            # Auto-generated: all input structs
    +-- output.rs           # Auto-generated: all output structs
```

**Dependencies:** `serde`, `serde_json`

### 7.2 `rustack-cloudwatch-core`

```
crates/rustack-cloudwatch-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # CloudWatchConfig
    +-- handler.rs          # CloudWatchHandler trait (all operation dispatch)
    +-- provider.rs         # RustackCloudWatch (main provider, all operation handlers)
    +-- metric_store.rs     # MetricStore: time-series storage for metric data points
    +-- alarm_store.rs      # AlarmStore: alarm configuration and state management
    +-- dashboard_store.rs  # DashboardStore: dashboard JSON body storage
    +-- aggregation.rs      # Statistical aggregation: Sum, Average, Min, Max, SampleCount
    +-- dimensions.rs       # Dimension normalization and matching
    +-- alarm_history.rs    # AlarmHistoryStore: alarm state change history
    +-- validation.rs       # Input validation (namespace, metric name, dimensions, etc.)
```

**Dependencies:** `rustack-core`, `rustack-cloudwatch-model`, `dashmap`, `serde_json`, `tracing`, `chrono`

### 7.3 `rustack-cloudwatch-http`

```
crates/rustack-cloudwatch-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # Action= parameter dispatch to CloudWatchOperation
    +-- service.rs          # CloudWatchHttpService (hyper Service impl)
    +-- dispatch.rs         # CloudWatchHandler trait + operation dispatch
    +-- body.rs             # Response body type
    +-- response.rs         # XML response construction
    +-- request.rs          # Form-urlencoded request parsing (nested .member.N)
```

**Dependencies:** `rustack-cloudwatch-model`, `rustack-auth`, `hyper`, `http`, `quick-xml`, `serde_urlencoded`, `bytes`

This crate is structurally similar to `rustack-sns-http`. The router parses `Action=<Op>` from the form body and dispatches to the handler.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-cloudwatch-model = { path = "crates/rustack-cloudwatch-model" }
rustack-cloudwatch-http = { path = "crates/rustack-cloudwatch-http" }
rustack-cloudwatch-core = { path = "crates/rustack-cloudwatch-core" }
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// CloudWatch Metrics operation router.
///
/// Parses the `Action=<Op>` parameter from the form-urlencoded request body
/// to determine the operation.
pub struct CloudWatchRouter;

impl CloudWatchRouter {
    /// Resolve an action name to a CloudWatch operation.
    pub fn resolve(action: &str) -> Result<CloudWatchOperation, CloudWatchError> {
        CloudWatchOperation::from_name(action)
            .ok_or_else(|| CloudWatchError::invalid_action(action))
    }
}

/// All recognized CloudWatch Metrics actions.
pub const CLOUDWATCH_ACTIONS: &[&str] = &[
    "PutMetricData", "GetMetricData", "GetMetricStatistics", "ListMetrics",
    "PutMetricAlarm", "DescribeAlarms", "DescribeAlarmsForMetric",
    "DeleteAlarms", "SetAlarmState",
    "EnableAlarmActions", "DisableAlarmActions",
    "DescribeAlarmHistory",
    "TagResource", "UntagResource", "ListTagsForResource",
    "PutCompositeAlarm",
    "PutDashboard", "GetDashboard", "DeleteDashboards", "ListDashboards",
    "PutInsightRule", "DeleteInsightRules", "DescribeInsightRules",
    "PutAnomalyDetector", "DescribeAnomalyDetectors", "DeleteAnomalyDetector",
    "PutManagedInsightRules",
    "PutMetricStream", "DeleteMetricStream", "ListMetricStreams", "GetMetricStream",
];
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// CloudWatch Metrics service router for the gateway.
///
/// Matches `POST /` requests with `Content-Type: application/x-www-form-urlencoded`
/// where the SigV4 signing service is `monitoring`.
pub struct CloudWatchServiceRouter<H: CloudWatchHandler> {
    inner: CloudWatchHttpService<H>,
}

impl<H: CloudWatchHandler> CloudWatchServiceRouter<H> {
    /// Wrap a [`CloudWatchHttpService`] in a router.
    pub fn new(inner: CloudWatchHttpService<H>) -> Self {
        Self { inner }
    }
}

impl<H: CloudWatchHandler> ServiceRouter for CloudWatchServiceRouter<H> {
    fn name(&self) -> &'static str {
        "cloudwatch"
    }

    /// CloudWatch Metrics matches form-urlencoded POST requests signed
    /// with the `monitoring` SigV4 service name.
    fn matches(&self, req: &http::Request<Incoming>) -> bool {
        if *req.method() != http::Method::POST {
            return false;
        }
        let is_form_encoded = req.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| ct.contains("x-www-form-urlencoded"));
        if !is_form_encoded {
            return false;
        }
        // Check SigV4 signing service name
        extract_sigv4_service(req)
            .is_some_and(|svc| svc == "monitoring")
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
```

### 8.3 Handler Trait

```rust
/// Trait that the CloudWatch business logic provider must implement.
pub trait CloudWatchHandler: Send + Sync + 'static {
    /// Handle a CloudWatch operation and produce an HTTP response.
    fn handle_operation(
        &self,
        op: CloudWatchOperation,
        params: FormParams,
    ) -> Pin<Box<dyn Future<Output = Result<
        http::Response<CloudWatchResponseBody>,
        CloudWatchError,
    >> + Send>>;
}

/// Parsed form parameters from the request body.
///
/// Provides methods for extracting flat parameters, lists (`.member.N`),
/// and nested structures (`.member.N.Field`).
pub struct FormParams {
    params: Vec<(String, String)>,
}

impl FormParams {
    /// Get a single parameter value.
    pub fn get(&self, key: &str) -> Option<&str>;

    /// Get a required parameter, returning an error if missing.
    pub fn require(&self, key: &str) -> Result<&str, CloudWatchError>;

    /// Get a list parameter (e.g., `Statistics.member.1`, `Statistics.member.2`).
    pub fn get_list(&self, prefix: &str) -> Vec<&str>;

    /// Get a list of structs (e.g., `MetricData.member.1.MetricName`,
    /// `MetricData.member.1.Value`).
    pub fn get_struct_list(&self, prefix: &str) -> Vec<FormParams>;

    /// Get all parameters with a given prefix as a sub-FormParams.
    pub fn with_prefix(&self, prefix: &str) -> FormParams;
}
```

### 8.4 XML Response Builder

```rust
/// Build an XML success response for a CloudWatch operation.
///
/// All CloudWatch responses follow the same envelope structure:
/// ```xml
/// <ActionResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
///   <ActionResult>
///     <!-- operation-specific content -->
///   </ActionResult>
///   <ResponseMetadata>
///     <RequestId>uuid</RequestId>
///   </ResponseMetadata>
/// </ActionResponse>
/// ```
pub fn build_xml_response(
    action: &str,
    result_body: &str,
    request_id: &str,
) -> String {
    format!(
        r#"<{action}Response xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <{action}Result>
    {result_body}
  </{action}Result>
  <ResponseMetadata>
    <RequestId>{request_id}</RequestId>
  </ResponseMetadata>
</{action}Response>"#
    )
}

/// Build an XML error response.
pub fn build_xml_error(
    error_type: &str,
    code: &str,
    message: &str,
    request_id: &str,
) -> String {
    format!(
        r#"<ErrorResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <Error>
    <Type>{error_type}</Type>
    <Code>{code}</Code>
    <Message>{message}</Message>
  </Error>
  <RequestId>{request_id}</RequestId>
</ErrorResponse>"#
    )
}
```

---

## 9. Storage Engine Design

### 9.1 Overview

The storage model consists of three independent stores:

1. **MetricStore** -- time-series data: metric data points keyed by (namespace, metric_name, dimensions)
2. **AlarmStore** -- alarm configurations and current state, keyed by alarm name
3. **DashboardStore** -- dashboard JSON bodies, keyed by dashboard name

All stores use `DashMap` for concurrent access.

### 9.2 Core Data Structures

```rust
/// Top-level metric store.
///
/// Metrics are uniquely identified by the combination of namespace, metric name,
/// and dimensions. Dimensions are normalized (sorted by name) for consistent lookup.
pub struct MetricStore {
    /// All metric series keyed by `MetricKey`.
    series: DashMap<MetricKey, MetricSeries>,
}

/// Unique identifier for a metric series.
///
/// Dimensions are sorted by name for consistent lookup regardless of insertion order.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct MetricKey {
    /// CloudWatch namespace (e.g., "AWS/EC2", "MyApp").
    pub namespace: String,
    /// Metric name (e.g., "CPUUtilization", "RequestCount").
    pub metric_name: String,
    /// Sorted dimensions. Order-independent: (A=1, B=2) == (B=2, A=1).
    pub dimensions: Vec<Dimension>,
}

impl MetricKey {
    /// Create a new MetricKey with dimensions normalized (sorted by name).
    pub fn new(
        namespace: String,
        metric_name: String,
        mut dimensions: Vec<Dimension>,
    ) -> Self {
        dimensions.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            namespace,
            metric_name,
            dimensions,
        }
    }
}

/// A single dimension key-value pair.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Dimension {
    pub name: String,
    pub value: String,
}

/// Time-series data for a single metric.
pub struct MetricSeries {
    /// Data points sorted by timestamp.
    /// Using BTreeMap keyed by timestamp (epoch milliseconds) for efficient range queries.
    pub data_points: BTreeMap<i64, Vec<DataPoint>>,
    /// Unit for this metric series (set by first PutMetricData call).
    pub unit: Option<StandardUnit>,
}

/// A single data point within a metric series.
#[derive(Debug, Clone)]
pub struct DataPoint {
    /// Timestamp in epoch milliseconds.
    pub timestamp: i64,
    /// Simple scalar value (mutually exclusive with statistic_values/values/counts).
    pub value: Option<f64>,
    /// Pre-aggregated statistics (from StatisticValues in PutMetricData).
    pub statistic_values: Option<StatisticSet>,
    /// Array of values (for high-cardinality data).
    pub values: Vec<f64>,
    /// Array of counts corresponding to values.
    pub counts: Vec<f64>,
    /// Unit for this data point.
    pub unit: Option<StandardUnit>,
}

/// Pre-aggregated statistic set (from PutMetricData StatisticValues).
#[derive(Debug, Clone)]
pub struct StatisticSet {
    pub sample_count: f64,
    pub sum: f64,
    pub minimum: f64,
    pub maximum: f64,
}
```

### 9.3 Alarm Store

```rust
/// Alarm store holding all alarm configurations and current state.
pub struct AlarmStore {
    /// Metric alarms keyed by alarm name.
    metric_alarms: DashMap<String, AlarmRecord>,
    /// Composite alarms keyed by alarm name.
    composite_alarms: DashMap<String, CompositeAlarmRecord>,
    /// Alarm history entries (most recent first).
    history: DashMap<String, Vec<AlarmHistoryEntry>>,
}

/// A metric alarm configuration and current state.
pub struct AlarmRecord {
    /// Alarm name (unique within account/region).
    pub alarm_name: String,
    /// Alarm ARN.
    pub alarm_arn: String,
    /// Description (optional).
    pub alarm_description: Option<String>,

    // -- Metric configuration --
    /// Namespace of the metric to watch.
    pub namespace: String,
    /// Name of the metric to watch.
    pub metric_name: String,
    /// Dimensions to filter the metric.
    pub dimensions: Vec<Dimension>,
    /// Statistic to evaluate (Sum, Average, Minimum, Maximum, SampleCount).
    pub statistic: Option<String>,
    /// Extended statistic (e.g., "p99") -- stored but not evaluated in MVP.
    pub extended_statistic: Option<String>,
    /// Period in seconds for metric evaluation.
    pub period: i32,
    /// Number of periods to evaluate.
    pub evaluation_periods: i32,
    /// Number of data points that must breach to trigger alarm.
    pub datapoints_to_alarm: Option<i32>,

    // -- Threshold --
    /// Comparison operator (GreaterThanThreshold, LessThanThreshold, etc.).
    pub comparison_operator: String,
    /// Threshold value.
    pub threshold: Option<f64>,
    /// Treat missing data as (missing, notBreaching, breaching, ignore).
    pub treat_missing_data: Option<String>,
    /// Evaluate low sample count percentiles (evaluate, ignore).
    pub evaluate_low_sample_count_percentile: Option<String>,

    // -- Actions --
    /// Actions to execute when state transitions to ALARM.
    pub alarm_actions: Vec<String>,
    /// Actions to execute when state transitions to OK.
    pub ok_actions: Vec<String>,
    /// Actions to execute when state transitions to INSUFFICIENT_DATA.
    pub insufficient_data_actions: Vec<String>,
    /// Whether actions are enabled.
    pub actions_enabled: bool,

    // -- State --
    /// Current alarm state (OK, ALARM, INSUFFICIENT_DATA).
    pub state_value: String,
    /// Reason for the current state.
    pub state_reason: String,
    /// JSON data providing additional detail for the state reason.
    pub state_reason_data: Option<String>,
    /// Timestamp of last state update (epoch seconds).
    pub state_updated_timestamp: f64,
    /// Timestamp of last state transition (only changes when state_value changes).
    pub state_transitioned_timestamp: f64,

    // -- Metadata --
    /// Tags on the alarm resource.
    pub tags: Vec<Tag>,
    /// Unit for the metric.
    pub unit: Option<String>,
    /// When the alarm configuration was last updated.
    pub alarm_configuration_updated_timestamp: f64,
}

/// A composite alarm (references other alarms via a rule expression).
pub struct CompositeAlarmRecord {
    /// Alarm name (unique within account/region).
    pub alarm_name: String,
    /// Alarm ARN.
    pub alarm_arn: String,
    /// Description.
    pub alarm_description: Option<String>,
    /// Alarm rule expression (e.g., "ALARM(cpu-alarm) AND ALARM(mem-alarm)").
    pub alarm_rule: String,
    /// Actions (same as AlarmRecord).
    pub alarm_actions: Vec<String>,
    pub ok_actions: Vec<String>,
    pub insufficient_data_actions: Vec<String>,
    pub actions_enabled: bool,
    /// Current state.
    pub state_value: String,
    pub state_reason: String,
    pub state_reason_data: Option<String>,
    pub state_updated_timestamp: f64,
    pub state_transitioned_timestamp: f64,
    /// Tags.
    pub tags: Vec<Tag>,
    /// Action suppressor configuration.
    pub actions_suppressor: Option<String>,
    pub actions_suppressor_wait_period: Option<i32>,
    pub actions_suppressor_extension_period: Option<i32>,
    /// Alarm configuration updated timestamp.
    pub alarm_configuration_updated_timestamp: f64,
}

/// Alarm history entry.
pub struct AlarmHistoryEntry {
    /// Alarm name.
    pub alarm_name: String,
    /// Alarm type (MetricAlarm or CompositeAlarm).
    pub alarm_type: String,
    /// History item type (ConfigurationUpdate, StateUpdate, Action).
    pub history_item_type: String,
    /// Timestamp (epoch seconds).
    pub timestamp: f64,
    /// Human-readable summary.
    pub history_summary: String,
    /// JSON detail.
    pub history_data: String,
}
```

### 9.4 Dashboard Store

```rust
/// Dashboard store holding dashboard JSON bodies.
pub struct DashboardStore {
    /// Dashboards keyed by dashboard name.
    dashboards: DashMap<String, DashboardRecord>,
}

/// A stored dashboard.
pub struct DashboardRecord {
    /// Dashboard name.
    pub dashboard_name: String,
    /// Dashboard ARN.
    pub dashboard_arn: String,
    /// Dashboard body (JSON string).
    pub dashboard_body: String,
    /// Last modified timestamp (epoch seconds).
    pub last_modified: f64,
    /// Size in bytes.
    pub size: i64,
}
```

### 9.5 Dimension Normalization

Metrics are uniquely identified by (namespace, metric_name, dimensions). Dimensions are key-value pairs and order-independent. We normalize dimension sets by sorting them by name for consistent lookup:

```rust
/// Normalize a set of dimensions for consistent storage and lookup.
///
/// Dimensions are sorted by name. This ensures that
/// `[{Name: "B", Value: "2"}, {Name: "A", Value: "1"}]` and
/// `[{Name: "A", Value: "1"}, {Name: "B", Value: "2"}]`
/// produce the same MetricKey.
pub fn normalize_dimensions(mut dimensions: Vec<Dimension>) -> Vec<Dimension> {
    dimensions.sort_by(|a, b| a.name.cmp(&b.name));
    dimensions.dedup_by(|a, b| a.name == b.name);
    dimensions
}
```

### 9.6 Data Retention

Real AWS CloudWatch retains metric data for up to 15 months with resolution-based tiers (1-second data for 3 hours, 1-minute for 15 days, 5-minute for 63 days, 1-hour for 15 months). For local development, we keep all data in memory with a configurable maximum retention period:

```rust
pub struct MetricStoreConfig {
    /// Maximum retention period in seconds. Data older than this is
    /// eligible for cleanup. Default: 86400 (24 hours).
    /// Set to 0 for unlimited retention (memory-bound only).
    pub max_retention_seconds: u64,
    /// Maximum number of data points per metric series.
    /// Default: 100_000. When exceeded, oldest points are removed.
    pub max_points_per_series: usize,
}

impl Default for MetricStoreConfig {
    fn default() -> Self {
        Self {
            max_retention_seconds: 86400, // 24 hours
            max_points_per_series: 100_000,
        }
    }
}
```

### 9.7 Metric Aggregation Engine

The aggregation engine is the core algorithmic component. When `GetMetricStatistics` is called, raw data points are grouped into period-aligned time buckets and the requested statistics are computed for each bucket:

```rust
/// Aggregate data points into period-aligned buckets and compute statistics.
///
/// # Arguments
/// * `data_points` - Raw data points within the requested time range
/// * `start_time` - Start of the query range (epoch seconds)
/// * `end_time` - End of the query range (epoch seconds)
/// * `period` - Aggregation period in seconds (must be >= 60, multiple of 60)
/// * `statistics` - Which statistics to compute (Sum, Average, Minimum, Maximum, SampleCount)
///
/// # Returns
/// A vector of aggregated datapoints, one per non-empty period bucket.
pub fn aggregate_statistics(
    data_points: &BTreeMap<i64, Vec<DataPoint>>,
    start_time: i64,
    end_time: i64,
    period: i64,
    statistics: &[Statistic],
) -> Vec<AggregatedDatapoint> {
    // Convert start/end to milliseconds for BTreeMap range queries
    let start_ms = start_time * 1000;
    let end_ms = end_time * 1000;

    // Collect data points in range
    let points_in_range: Vec<&DataPoint> = data_points
        .range(start_ms..end_ms)
        .flat_map(|(_, points)| points.iter())
        .collect();

    if points_in_range.is_empty() {
        return Vec::new();
    }

    // Group points into period-aligned buckets.
    // Bucket start = floor(timestamp / period) * period
    let period_ms = period * 1000;
    let mut buckets: BTreeMap<i64, Vec<&DataPoint>> = BTreeMap::new();
    for point in &points_in_range {
        let bucket_start = (point.timestamp / period_ms) * period_ms;
        buckets.entry(bucket_start).or_default().push(point);
    }

    // Compute statistics for each bucket
    let mut results = Vec::with_capacity(buckets.len());
    for (bucket_start, bucket_points) in &buckets {
        let aggregated = compute_bucket_statistics(
            *bucket_start / 1000, // convert back to epoch seconds for output
            bucket_points,
            statistics,
        );
        results.push(aggregated);
    }

    results
}

/// Compute statistics for a single period bucket.
fn compute_bucket_statistics(
    timestamp: i64,
    points: &[&DataPoint],
    statistics: &[Statistic],
) -> AggregatedDatapoint {
    // Expand all data points into (value, count) pairs.
    // Simple values have count=1. StatisticSets contribute pre-aggregated stats.
    // Values/Counts arrays contribute multiple (value, count) pairs.
    let mut sum = 0.0_f64;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut sample_count = 0.0_f64;

    for point in points {
        if let Some(ss) = &point.statistic_values {
            // Pre-aggregated statistic set
            sum += ss.sum;
            if ss.minimum < min { min = ss.minimum; }
            if ss.maximum > max { max = ss.maximum; }
            sample_count += ss.sample_count;
        } else if !point.values.is_empty() {
            // Values/Counts arrays
            for (i, val) in point.values.iter().enumerate() {
                let count = point.counts.get(i).copied().unwrap_or(1.0);
                sum += val * count;
                if *val < min { min = *val; }
                if *val > max { max = *val; }
                sample_count += count;
            }
        } else if let Some(val) = point.value {
            // Simple scalar value
            sum += val;
            if val < min { min = val; }
            if val > max { max = val; }
            sample_count += 1.0;
        }
    }

    let average = if sample_count > 0.0 { sum / sample_count } else { 0.0 };

    AggregatedDatapoint {
        timestamp,
        sum: if statistics.contains(&Statistic::Sum) { Some(sum) } else { None },
        average: if statistics.contains(&Statistic::Average) { Some(average) } else { None },
        minimum: if statistics.contains(&Statistic::Minimum) { Some(min) } else { None },
        maximum: if statistics.contains(&Statistic::Maximum) { Some(max) } else { None },
        sample_count: if statistics.contains(&Statistic::SampleCount) { Some(sample_count) } else { None },
        unit: points.first().and_then(|p| p.unit.clone()),
    }
}

/// Result of aggregating a period bucket.
#[derive(Debug, Clone)]
pub struct AggregatedDatapoint {
    pub timestamp: i64,
    pub sum: Option<f64>,
    pub average: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub sample_count: Option<f64>,
    pub unit: Option<StandardUnit>,
}
```

### 9.8 Concurrency Model

CloudWatch Metrics has no real-time constraints or streaming (unlike Kinesis). A `DashMap` provides sufficient concurrent access:

- **Writes** (PutMetricData): lock-free concurrent writes to different metric series; per-entry lock for same series
- **Reads** (GetMetricStatistics, GetMetricData, ListMetrics): lock-free concurrent reads
- **Alarm state updates** (SetAlarmState, PutMetricAlarm): per-alarm entry lock

The only potential background processing is alarm evaluation, which is optional for MVP. If implemented in future phases, the alarm evaluator runs on a separate Tokio task with configurable evaluation intervals.

---

## 10. Core Business Logic

### 10.1 Provider

```rust
/// Main CloudWatch Metrics provider implementing all operations.
pub struct RustackCloudWatch {
    pub(crate) metric_store: Arc<MetricStore>,
    pub(crate) alarm_store: Arc<AlarmStore>,
    pub(crate) dashboard_store: Arc<DashboardStore>,
    pub(crate) config: Arc<CloudWatchConfig>,
}

impl RustackCloudWatch {
    pub fn new(config: CloudWatchConfig) -> Self {
        Self {
            metric_store: Arc::new(MetricStore::new(config.metric_store_config.clone())),
            alarm_store: Arc::new(AlarmStore::new()),
            dashboard_store: Arc::new(DashboardStore::new()),
            config: Arc::new(config),
        }
    }
}
```

### 10.2 Phase 0 Operations (~15 operations)

#### PutMetricData

Accept metric data points and store them in the time-series engine.

1. Validate namespace (1-255 chars, no `:` prefix reserved for AWS)
2. For each `MetricDatum` in the request:
   a. Validate metric name (1-255 chars)
   b. Validate dimensions (max 30 per metric, name 1-255 chars, value 1-1024 chars)
   c. Normalize dimensions (sort by name)
   d. Construct `MetricKey` from (namespace, metric_name, dimensions)
   e. Parse timestamp (default to current time if absent)
   f. Validate exactly one of: `Value`, `StatisticValues`, `Values`/`Counts`
   g. If `Values` and `Counts` provided, validate equal length
   h. Store data point in MetricStore
3. Return empty success response (PutMetricData has no result body)

```rust
impl RustackCloudWatch {
    pub fn put_metric_data(
        &self,
        input: PutMetricDataInput,
    ) -> Result<PutMetricDataOutput, CloudWatchError> {
        let namespace = &input.namespace;
        validate_namespace(namespace)?;

        for datum in &input.metric_data {
            validate_metric_name(&datum.metric_name)?;
            let dimensions = normalize_dimensions(
                datum.dimensions.clone().unwrap_or_default()
            );
            validate_dimensions(&dimensions)?;

            let key = MetricKey::new(
                namespace.clone(),
                datum.metric_name.clone(),
                dimensions,
            );

            let timestamp = datum.timestamp
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

            let data_point = DataPoint {
                timestamp,
                value: datum.value,
                statistic_values: datum.statistic_values.clone(),
                values: datum.values.clone().unwrap_or_default(),
                counts: datum.counts.clone().unwrap_or_default(),
                unit: datum.unit.clone(),
            };

            self.metric_store.insert(key, data_point);
        }

        Ok(PutMetricDataOutput {})
    }
}
```

#### GetMetricStatistics

Query metrics by namespace/name/dimensions/time range and compute statistics over period-aligned buckets.

1. Validate required params: Namespace, MetricName, StartTime, EndTime, Period
2. Validate Period >= 60 and is a multiple of 60 (for standard resolution)
3. Validate at least one of Statistics or ExtendedStatistics
4. Normalize dimensions
5. Construct MetricKey and look up the metric series
6. Use aggregation engine to compute statistics over the time range
7. Return `{ Label, Datapoints }`

```rust
impl RustackCloudWatch {
    pub fn get_metric_statistics(
        &self,
        input: GetMetricStatisticsInput,
    ) -> Result<GetMetricStatisticsOutput, CloudWatchError> {
        let key = MetricKey::new(
            input.namespace.clone(),
            input.metric_name.clone(),
            normalize_dimensions(input.dimensions.clone().unwrap_or_default()),
        );

        let datapoints = match self.metric_store.get(&key) {
            Some(series) => aggregate_statistics(
                &series.data_points,
                input.start_time,
                input.end_time,
                input.period as i64,
                &input.statistics,
            ),
            None => Vec::new(),
        };

        Ok(GetMetricStatisticsOutput {
            label: Some(input.metric_name),
            datapoints,
        })
    }
}
```

#### GetMetricData

More powerful metric query using `MetricDataQueries`. Each query can reference a `MetricStat` (namespace + metric name + dimensions + period + stat) or an `Expression` (math on other queries). For MVP, only `MetricStat` queries are supported; `Expression` queries return an error.

1. Validate at least one MetricDataQuery
2. For each query:
   a. If `MetricStat` is present: execute as GetMetricStatistics equivalent
   b. If `Expression` is present: return `InvalidParameterValue` (not supported in MVP)
3. Paginate results with MaxDatapoints and NextToken
4. Return `{ MetricDataResults, NextToken, Messages }`

#### ListMetrics

Enumerate all known metric name/namespace/dimension combinations.

1. Iterate all MetricKeys in the metric store
2. Apply optional filters:
   - `Namespace` -- exact match
   - `MetricName` -- exact match
   - `Dimensions` -- each filter dimension must match (name + optional value)
   - `RecentlyActive` -- if "PT3H", only include metrics with data points in last 3 hours
3. Paginate with `NextToken`
4. Return `{ Metrics: [{ Namespace, MetricName, Dimensions }], NextToken }`

#### PutMetricAlarm

Store an alarm configuration.

1. Validate alarm name (1-255 chars)
2. Validate namespace, metric name, dimensions
3. Validate comparison operator (GreaterThanOrEqualToThreshold, GreaterThanThreshold, LessThanThreshold, LessThanOrEqualToThreshold, LessThanLowerOrGreaterThanUpperThreshold, LessThanLowerThreshold, GreaterThanUpperThreshold)
4. Validate evaluation periods (>= 1)
5. Validate period (>= 10 seconds for high-resolution, >= 60 for standard, multiple of 60 for >= 60)
6. Generate alarm ARN: `arn:aws:cloudwatch:{region}:{account}:alarm:{alarm_name}`
7. If alarm exists, update it; if new, create with initial state INSUFFICIENT_DATA
8. Record history entry (ConfigurationUpdate)
9. If tags provided, store tags
10. Return empty success response

```rust
impl RustackCloudWatch {
    pub fn put_metric_alarm(
        &self,
        input: PutMetricAlarmInput,
    ) -> Result<PutMetricAlarmOutput, CloudWatchError> {
        validate_alarm_name(&input.alarm_name)?;
        let now = now_epoch_seconds();

        let existing = self.alarm_store.get(&input.alarm_name);
        let is_update = existing.is_some();

        let state_value = existing
            .as_ref()
            .map(|a| a.state_value.clone())
            .unwrap_or_else(|| "INSUFFICIENT_DATA".to_string());
        let state_reason = existing
            .as_ref()
            .map(|a| a.state_reason.clone())
            .unwrap_or_else(|| "Unchecked: Initial alarm creation".to_string());

        let alarm = AlarmRecord {
            alarm_name: input.alarm_name.clone(),
            alarm_arn: alarm_arn(
                &self.config.default_region,
                &self.config.default_account_id,
                &input.alarm_name,
            ),
            alarm_description: input.alarm_description,
            namespace: input.namespace,
            metric_name: input.metric_name,
            dimensions: normalize_dimensions(
                input.dimensions.unwrap_or_default()
            ),
            statistic: input.statistic,
            extended_statistic: input.extended_statistic,
            period: input.period,
            evaluation_periods: input.evaluation_periods,
            datapoints_to_alarm: input.datapoints_to_alarm,
            comparison_operator: input.comparison_operator,
            threshold: input.threshold,
            treat_missing_data: input.treat_missing_data,
            evaluate_low_sample_count_percentile:
                input.evaluate_low_sample_count_percentile,
            alarm_actions: input.alarm_actions.unwrap_or_default(),
            ok_actions: input.ok_actions.unwrap_or_default(),
            insufficient_data_actions: input
                .insufficient_data_actions
                .unwrap_or_default(),
            actions_enabled: input.actions_enabled.unwrap_or(true),
            state_value,
            state_reason,
            state_reason_data: None,
            state_updated_timestamp: now,
            state_transitioned_timestamp: now,
            tags: input.tags.unwrap_or_default(),
            unit: input.unit,
            alarm_configuration_updated_timestamp: now,
        };

        // Record history
        let history_type = if is_update {
            "Configuration updated"
        } else {
            "Alarm created"
        };
        self.alarm_store.record_history(
            &alarm.alarm_name,
            "MetricAlarm",
            "ConfigurationUpdate",
            now,
            history_type,
        );

        self.alarm_store.put_metric_alarm(alarm);
        Ok(PutMetricAlarmOutput {})
    }
}
```

#### DescribeAlarms

List alarms with optional filtering.

1. Apply filters:
   - `AlarmNames` -- exact match list
   - `AlarmNamePrefix` -- prefix match
   - `AlarmTypes` -- filter by MetricAlarm and/or CompositeAlarm
   - `StateValue` -- filter by current state (OK, ALARM, INSUFFICIENT_DATA)
   - `ActionPrefix` -- filter by action ARN prefix
   - `ParentsOfAlarmName` -- composite alarms that reference this alarm (Phase 1)
   - `ChildrenOfAlarmName` -- alarms referenced by this composite alarm (Phase 1)
2. Paginate with MaxRecords (1-100) and NextToken
3. Return `{ MetricAlarms, CompositeAlarms, NextToken }`

#### DescribeAlarmsForMetric

List alarms that monitor a specific metric.

1. Require Namespace and MetricName
2. Filter by optional Dimensions, Period, Statistic, ExtendedStatistic, Unit
3. Return `{ MetricAlarms }`

#### DeleteAlarms

Delete one or more alarms by name.

1. For each alarm name:
   a. If alarm exists, remove it from the store
   b. If alarm does not exist, return `ResourceNotFound` error
2. Return empty success response

#### SetAlarmState

Manually set the state of an alarm. This is the primary mechanism for alarm state changes in MVP (before automatic evaluation is implemented).

1. Resolve alarm by name
2. Validate new state value (OK, ALARM, INSUFFICIENT_DATA)
3. If state changed from previous state:
   a. Record history entry (StateUpdate)
   b. Trigger alarm actions based on new state:
      - ALARM -> execute alarm_actions
      - OK -> execute ok_actions
      - INSUFFICIENT_DATA -> execute insufficient_data_actions
   c. For MVP, actions are recorded in history but not executed
4. Update alarm state, reason, and timestamps
5. Return empty success response

```rust
impl RustackCloudWatch {
    pub fn set_alarm_state(
        &self,
        input: SetAlarmStateInput,
    ) -> Result<SetAlarmStateOutput, CloudWatchError> {
        let alarm_name = &input.alarm_name;
        let new_state = &input.state_value;
        let now = now_epoch_seconds();

        let mut alarm = self.alarm_store.get_mut(alarm_name)
            .ok_or_else(|| CloudWatchError::resource_not_found(
                &format!("The alarm '{}' does not exist.", alarm_name),
            ))?;

        let old_state = alarm.state_value.clone();
        let state_changed = old_state != *new_state;

        alarm.state_value = new_state.clone();
        alarm.state_reason = input.state_reason.clone();
        alarm.state_reason_data = input.state_reason_data.clone();
        alarm.state_updated_timestamp = now;

        if state_changed {
            alarm.state_transitioned_timestamp = now;

            // Record history
            self.alarm_store.record_history(
                alarm_name,
                "MetricAlarm",
                "StateUpdate",
                now,
                &format!(
                    "Alarm updated from {} to {}",
                    old_state, new_state
                ),
            );

            // Future: trigger alarm actions here
            // let actions = match new_state.as_str() {
            //     "ALARM" => &alarm.alarm_actions,
            //     "OK" => &alarm.ok_actions,
            //     "INSUFFICIENT_DATA" => &alarm.insufficient_data_actions,
            //     _ => &Vec::new(),
            // };
            // for action_arn in actions {
            //     self.execute_action(action_arn).await;
            // }
        }

        Ok(SetAlarmStateOutput {})
    }
}
```

#### EnableAlarmActions / DisableAlarmActions

Toggle whether alarm actions are executed when state changes.

1. For each alarm name in the request:
   a. Resolve alarm
   b. Set `actions_enabled = true` (enable) or `actions_enabled = false` (disable)
2. Return empty success response

#### DescribeAlarmHistory

Query alarm state change history.

1. Apply filters:
   - `AlarmName` -- specific alarm
   - `AlarmTypes` -- MetricAlarm and/or CompositeAlarm
   - `HistoryItemType` -- ConfigurationUpdate, StateUpdate, Action
   - `StartDate` / `EndDate` -- time range
2. Paginate with MaxRecords and NextToken
3. Return `{ AlarmHistoryItems, NextToken }`

#### TagResource / UntagResource / ListTagsForResource

Standard tag operations for CloudWatch resources (alarms, insight rules).

1. Parse the resource ARN to determine resource type and name
2. For TagResource: merge new tags (overwrite existing keys)
3. For UntagResource: remove tags by key
4. For ListTagsForResource: return all tags
5. Enforce 50-tag limit per resource

### 10.3 Phase 1 Operations (~8 operations)

#### PutCompositeAlarm

Store a composite alarm configuration. Composite alarms reference other alarms via a rule expression (e.g., `ALARM("cpu-alarm") AND ALARM("mem-alarm")`).

1. Validate alarm name and rule expression syntax
2. Store composite alarm record with initial state INSUFFICIENT_DATA
3. For MVP, do not evaluate the rule expression; state changes only via SetAlarmState
4. Return empty success response

#### PutDashboard / GetDashboard / DeleteDashboards / ListDashboards

Dashboard operations are simple JSON blob storage:

1. **PutDashboard**: Store `DashboardBody` (JSON string) keyed by `DashboardName`. Validate that `DashboardBody` is valid JSON. Return `{ DashboardValidationMessages }` (empty for valid JSON).
2. **GetDashboard**: Return `{ DashboardName, DashboardArn, DashboardBody }`.
3. **DeleteDashboards**: Remove dashboards by name list. Return empty success.
4. **ListDashboards**: List all dashboards with optional `DashboardNamePrefix` filter. Return `{ DashboardEntries, NextToken }`.

#### PutInsightRule / DeleteInsightRules / DescribeInsightRules

Insight rule operations are metadata storage only (no actual rule evaluation):

1. **PutInsightRule**: Store the rule name, definition, and state. Return empty success.
2. **DeleteInsightRules**: Remove rules by name list. Return `{ Failures }` (empty on success).
3. **DescribeInsightRules**: List rules with optional filtering. Return `{ InsightRules, NextToken }`.

### 10.4 Phase 2 Operations (~8 operations)

#### PutAnomalyDetector / DescribeAnomalyDetectors / DeleteAnomalyDetector

Anomaly detector operations are metadata storage only (no model training):

1. **PutAnomalyDetector**: Store the detector configuration (namespace, metric name, dimensions, stat, configuration). Return empty success.
2. **DescribeAnomalyDetectors**: List detectors with optional filtering. Return `{ AnomalyDetectors, NextToken }`.
3. **DeleteAnomalyDetector**: Remove a detector. Return empty success.

#### PutMetricStream / DeleteMetricStream / ListMetricStreams / GetMetricStream

Metric stream operations are metadata storage only (no actual streaming):

1. **PutMetricStream**: Store stream configuration (name, Firehose ARN, filters, output format). Return `{ Arn }`.
2. **DeleteMetricStream**: Remove a stream by name. Return empty success.
3. **ListMetricStreams**: List streams. Return `{ Entries, NextToken }`.
4. **GetMetricStream**: Get stream details. Return `{ Name, Arn, FirehoseArn, ... }`.

#### PutManagedInsightRules

Store managed insight rule configuration. Return `{ Failures }` (empty on success).

### 10.5 Validation Rules

| Field | Rule |
|-------|------|
| Namespace | 1-255 chars; cannot start with `AWS/` (reserved for AWS services, but we accept it for local dev) |
| Metric name | 1-255 chars |
| Dimension name | 1-255 chars |
| Dimension value | 1-1024 chars |
| Dimensions per metric | Max 30 |
| MetricData per PutMetricData | Max 1000 (per batch, but we accept more for local dev) |
| Values array | Max 150 values per datum |
| Alarm name | 1-255 chars |
| Period | >= 10 for high-res, >= 60 for standard; multiple of 60 for >= 60 |
| Evaluation periods | >= 1 |
| Dashboard name | 1-255 chars |
| Dashboard body | Valid JSON, max 1MB |
| Tags per resource | Max 50 |
| Tag key | 1-128 chars |
| Tag value | 0-256 chars |

---

## 11. Error Handling

### 11.1 Error Types

```rust
/// CloudWatch error codes matching the AWS API.
#[derive(Debug, Clone)]
pub enum CloudWatchErrorCode {
    /// The named resource does not exist.
    ResourceNotFound,
    /// The named resource already exists.
    ResourceAlreadyExists,
    /// The value of an input parameter is bad or out-of-range.
    InvalidParameterValue,
    /// An input parameter that is required is missing.
    MissingRequiredParameter,
    /// The named parameter combination is invalid.
    InvalidParameterCombination,
    /// The next token specified is invalid.
    InvalidNextToken,
    /// Request processing has exceeded the request-limit of the service.
    LimitExceeded,
    /// An internal service error occurred.
    InternalServiceError,
    /// A format error in the input.
    InvalidFormatFault,
    /// The dashboard body is not valid JSON.
    DashboardInvalidInput,
    /// The operation is not valid for the current state.
    InvalidAction,
}
```

### 11.2 Error Mapping

```rust
impl CloudWatchError {
    /// Map to HTTP status code, error type, code string, and message.
    pub fn to_error_response(&self) -> (u16, &'static str, &'static str, String) {
        match &self.code {
            CloudWatchErrorCode::ResourceNotFound =>
                (404, "Sender", "ResourceNotFound", self.message.clone()),
            CloudWatchErrorCode::ResourceAlreadyExists =>
                (409, "Sender", "ResourceAlreadyExists", self.message.clone()),
            CloudWatchErrorCode::InvalidParameterValue =>
                (400, "Sender", "InvalidParameterValue", self.message.clone()),
            CloudWatchErrorCode::MissingRequiredParameter =>
                (400, "Sender", "MissingParameter", self.message.clone()),
            CloudWatchErrorCode::InvalidParameterCombination =>
                (400, "Sender", "InvalidParameterCombination", self.message.clone()),
            CloudWatchErrorCode::InvalidNextToken =>
                (400, "Sender", "InvalidNextToken", self.message.clone()),
            CloudWatchErrorCode::LimitExceeded =>
                (400, "Sender", "LimitExceeded", self.message.clone()),
            CloudWatchErrorCode::InternalServiceError =>
                (500, "Receiver", "InternalServiceError", self.message.clone()),
            CloudWatchErrorCode::DashboardInvalidInput =>
                (400, "Sender", "InvalidParameterInput", self.message.clone()),
            CloudWatchErrorCode::InvalidFormatFault =>
                (400, "Sender", "InvalidFormat", self.message.clone()),
            CloudWatchErrorCode::InvalidAction =>
                (400, "Sender", "InvalidAction", self.message.clone()),
        }
    }
}
```

### 11.3 Error Response Format

CloudWatch errors follow the standard awsQuery XML error format (same as SNS):

```xml
<ErrorResponse xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
  <Error>
    <Type>Sender</Type>
    <Code>ResourceNotFound</Code>
    <Message>The alarm 'my-alarm' does not exist.</Message>
  </Error>
  <RequestId>d74b8436-ae13-5ab4-a9ff-ce54dfea72a0</RequestId>
</ErrorResponse>
```

The `<Type>` field is either `Sender` (client error, 4xx) or `Receiver` (server error, 5xx).

---

## 12. Server Integration

### 12.1 Feature Gate

CloudWatch Metrics support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "sns", "events", "logs", "kms", "kinesis", "secretsmanager", "cloudwatch"]
cloudwatch = ["dep:rustack-cloudwatch-core", "dep:rustack-cloudwatch-http"]
```

### 12.2 Gateway Registration

CloudWatch Metrics is registered in the gateway. Because it uses awsQuery (same as SNS), it must be registered **before** SNS so that the SigV4 service name check (`monitoring`) is evaluated first:

```rust
// In gateway setup (build_services function)

// CloudWatch Metrics must be before SNS -- both use awsQuery.
// CW Metrics matches on SigV4 service=monitoring, SNS on everything else form-urlencoded.
#[cfg(feature = "cloudwatch")]
if is_enabled("cloudwatch") {
    let cw_config = CloudWatchConfig::from_env();
    info!(
        cloudwatch_skip_signature_validation = cw_config.skip_signature_validation,
        "initializing CloudWatch Metrics service",
    );
    let cw_provider = RustackCloudWatch::new(cw_config.clone());
    let cw_handler = RustackCloudWatchHandler::new(Arc::new(cw_provider));
    let cw_http_config = build_cloudwatch_http_config(&cw_config);
    let cw_service = CloudWatchHttpService::new(Arc::new(cw_handler), cw_http_config);
    services.push(Box::new(service::CloudWatchServiceRouter::new(cw_service)));
}

// SNS (register after CloudWatch Metrics)
#[cfg(feature = "sns")]
if is_enabled("sns") {
    // ... existing SNS registration ...
}
```

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
        "events": "available",
        "logs": "available",
        "kms": "available",
        "kinesis": "available",
        "secretsmanager": "available",
        "cloudwatch": "available"
    },
    "version": "0.4.0"
}
```

### 12.4 Configuration

```rust
pub struct CloudWatchConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
    /// Metric store configuration.
    pub metric_store_config: MetricStoreConfig,
}

impl CloudWatchConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool(
                "CLOUDWATCH_SKIP_SIGNATURE_VALIDATION",
                true,
            ),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
            metric_store_config: MetricStoreConfig {
                max_retention_seconds: env_u64(
                    "CLOUDWATCH_MAX_RETENTION_SECONDS",
                    86400,
                ),
                max_points_per_series: env_usize(
                    "CLOUDWATCH_MAX_POINTS_PER_SERIES",
                    100_000,
                ),
            },
        }
    }
}
```

### 12.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `CLOUDWATCH_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for CloudWatch Metrics |
| `CLOUDWATCH_MAX_RETENTION_SECONDS` | `86400` | Maximum metric data retention (seconds) |
| `CLOUDWATCH_MAX_POINTS_PER_SERIES` | `100000` | Maximum data points per metric series |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

### 12.6 Raw Metrics Endpoint

Following LocalStack's convention, provide a diagnostic endpoint for inspecting raw metric data:

```
GET /_aws/cloudwatch/metrics/raw?namespace=MyApp&metric_name=RequestCount
```

Returns JSON with all stored data points for the specified metric (not part of the AWS API, but useful for debugging and testing).

### 12.7 Docker Image / GitHub Action

The existing Docker image and GitHub Action gain CloudWatch Metrics support automatically when the feature is enabled. The GitHub Action `action.yml` should be updated to list `cloudwatch` as a supported service.

---

## 13. Testing Strategy

### 13.1 Unit Tests

Each module tested in isolation:

- **Metric storage**: insert data points, range queries, data point expiry, max points enforcement
- **Dimension normalization**: sort order independence, dedup, matching with partial dimensions
- **Aggregation engine**: Sum/Average/Min/Max/SampleCount computation, period alignment, empty buckets, pre-aggregated StatisticSets, Values/Counts arrays, mixed data point types
- **Alarm management**: create/update/delete, state transitions, history recording, actions_enabled toggle
- **Dashboard CRUD**: create, update, get, delete, list with prefix filter
- **Form parameter parsing**: flat params, `.member.N` lists, nested `.member.N.Field` structures, URL decoding
- **XML response generation**: correct envelope structure, namespace, escaping
- **Validation**: namespace format, metric name, dimension limits, period validation, alarm name

### 13.2 Integration Tests with aws-sdk-rust

```rust
// tests/integration/cloudwatch_tests.rs
#[tokio::test]
#[ignore]
async fn test_should_put_and_get_metric_data() {
    let client = aws_sdk_cloudwatch::Client::new(&config);
    // PutMetricData, GetMetricStatistics round-trip
    // Verify Sum, Average, SampleCount are correct
}

#[tokio::test]
#[ignore]
async fn test_should_list_metrics_with_filters() {
    // PutMetricData with multiple namespaces and dimensions
    // ListMetrics with Namespace, MetricName, Dimensions filters
}

#[tokio::test]
#[ignore]
async fn test_should_manage_metric_alarms() {
    // PutMetricAlarm, DescribeAlarms, SetAlarmState,
    // DescribeAlarmHistory, DeleteAlarms
}

#[tokio::test]
#[ignore]
async fn test_should_manage_alarm_actions() {
    // PutMetricAlarm with alarm/ok/insufficient_data actions
    // EnableAlarmActions, DisableAlarmActions
    // Verify via DescribeAlarms
}

#[tokio::test]
#[ignore]
async fn test_should_manage_dashboards() {
    // PutDashboard, GetDashboard, ListDashboards, DeleteDashboards
}

#[tokio::test]
#[ignore]
async fn test_should_manage_tags() {
    // PutMetricAlarm, TagResource, UntagResource, ListTagsForResource
}

#[tokio::test]
#[ignore]
async fn test_should_aggregate_statistics_correctly() {
    // PutMetricData with known values
    // GetMetricStatistics with different periods
    // Verify aggregation math is correct
}

#[tokio::test]
#[ignore]
async fn test_should_handle_statistic_values() {
    // PutMetricData with StatisticValues (pre-aggregated)
    // GetMetricStatistics and verify correct aggregation
}

#[tokio::test]
#[ignore]
async fn test_should_describe_alarms_for_metric() {
    // Create alarms for different metrics
    // DescribeAlarmsForMetric filters correctly
}
```

### 13.3 AWS CLI Smoke Tests

```bash
# Put metric data
aws cloudwatch put-metric-data \
    --namespace MyApp \
    --metric-name RequestCount \
    --value 42 \
    --unit Count \
    --dimensions Environment=Production,Service=API \
    --endpoint-url http://localhost:4566

# Get metric statistics
aws cloudwatch get-metric-statistics \
    --namespace MyApp \
    --metric-name RequestCount \
    --start-time 2026-03-19T00:00:00Z \
    --end-time 2026-03-20T00:00:00Z \
    --period 3600 \
    --statistics Sum Average \
    --dimensions Name=Environment,Value=Production \
    --endpoint-url http://localhost:4566

# List metrics
aws cloudwatch list-metrics \
    --namespace MyApp \
    --endpoint-url http://localhost:4566

# Put metric alarm
aws cloudwatch put-metric-alarm \
    --alarm-name HighRequestCount \
    --namespace MyApp \
    --metric-name RequestCount \
    --comparison-operator GreaterThanThreshold \
    --threshold 100 \
    --evaluation-periods 3 \
    --period 300 \
    --statistic Sum \
    --alarm-actions arn:aws:sns:us-east-1:000000000000:my-topic \
    --endpoint-url http://localhost:4566

# Describe alarms
aws cloudwatch describe-alarms \
    --endpoint-url http://localhost:4566

# Set alarm state
aws cloudwatch set-alarm-state \
    --alarm-name HighRequestCount \
    --state-value ALARM \
    --state-reason "Testing alarm trigger" \
    --endpoint-url http://localhost:4566

# Describe alarm history
aws cloudwatch describe-alarm-history \
    --alarm-name HighRequestCount \
    --endpoint-url http://localhost:4566

# Delete alarms
aws cloudwatch delete-alarms \
    --alarm-names HighRequestCount \
    --endpoint-url http://localhost:4566

# Put dashboard
aws cloudwatch put-dashboard \
    --dashboard-name MyDashboard \
    --dashboard-body '{"widgets":[]}' \
    --endpoint-url http://localhost:4566

# Get dashboard
aws cloudwatch get-dashboard \
    --dashboard-name MyDashboard \
    --endpoint-url http://localhost:4566
```

### 13.4 Third-Party Test Suites

#### 13.4.1 LocalStack CloudWatch Tests

**Location:** `vendors/localstack/localstack-core/localstack/services/cloudwatch/` (~531 lines provider)
**Coverage:** LocalStack wraps moto for CloudWatch. Key behaviors tested:
- PutMetricAlarm with alarm scheduling
- Alarm state changes trigger actions (SNS publish, Lambda invoke)
- PutCompositeAlarm (logged but not evaluated)
- GetMetricData, GetMetricStatistics
- Tag management
- Raw metrics endpoint at `/_aws/cloudwatch/metrics/raw`

**How to run:**
```makefile
test-cloudwatch-localstack:
	cd vendors/localstack && python -m pytest tests/aws/services/cloudwatch/ \
		-k "not alarm_action_invocation" \
		--endpoint-url=http://localhost:4566
```

#### 13.4.2 Terraform AWS Provider

**Resources tested:**
- `aws_cloudwatch_metric_alarm` -- PutMetricAlarm, DescribeAlarms, DeleteAlarms, tags
- `aws_cloudwatch_dashboard` -- PutDashboard, GetDashboard, DeleteDashboards
- `aws_cloudwatch_composite_alarm` -- PutCompositeAlarm (Phase 1)

**How to run:**
```bash
pip install terraform-local
tflocal init
tflocal apply
```

#### 13.4.3 Grafana CloudWatch Data Source

**Operations tested:** GetMetricData, GetMetricStatistics, ListMetrics
**How to run:** Configure Grafana CloudWatch data source with custom endpoint URL.

#### 13.4.4 Prometheus Remote Write Adapter

**Operations tested:** PutMetricData (high-throughput metric ingestion)
**How to run:** Configure AWS OTel Collector with CloudWatch exporter pointing at Rustack.

### 13.5 CI Integration

```yaml
# .github/workflows/cloudwatch-ci.yml
name: CloudWatch CI
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p rustack-cloudwatch-model
      - run: cargo test -p rustack-cloudwatch-core
      - run: cargo test -p rustack-cloudwatch-http

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
          aws cloudwatch put-metric-data \
            --namespace TestApp --metric-name TestMetric --value 42 \
            --endpoint-url http://localhost:4566
          aws cloudwatch list-metrics --namespace TestApp \
            --endpoint-url http://localhost:4566
          aws cloudwatch put-metric-alarm \
            --alarm-name TestAlarm --namespace TestApp \
            --metric-name TestMetric --comparison-operator GreaterThanThreshold \
            --threshold 10 --evaluation-periods 1 --period 60 --statistic Sum \
            --endpoint-url http://localhost:4566
          aws cloudwatch describe-alarms \
            --endpoint-url http://localhost:4566
      - run: |
          # Python integration tests
          pip install boto3 pytest
          pytest tests/integration/cloudwatch/ -v
```

---

## 14. Phased Implementation Plan

### Phase 0: Core Metrics and Alarms (4-5 days)

**Goal:** PutMetricData, GetMetricStatistics, ListMetrics, alarms CRUD, tags. Enough for `aws cloudwatch put-metric-data`, `get-metric-statistics`, `put-metric-alarm`, and `describe-alarms` to work. Terraform `aws_cloudwatch_metric_alarm` works.

1. **Day 1: Model + Scaffolding**
   - Create `codegen/services/cloudwatch.toml`
   - Add CloudWatch Smithy model (`monitoring-2010-08-01.json`) to codegen
   - Generate `rustack-cloudwatch-model` crate
   - Create `rustack-cloudwatch-core` and `rustack-cloudwatch-http` crate scaffolding
   - Implement `CloudWatchOperation` enum and router
   - Implement form parameter parser (reuse/adapt from SNS)

2. **Day 2: Metric Storage Engine**
   - Implement `MetricStore`, `MetricKey`, `MetricSeries`, `DataPoint`
   - Implement dimension normalization
   - Implement `PutMetricData` (accept and store data points)
   - Implement `ListMetrics` (enumerate stored metric keys)
   - Implement aggregation engine (`aggregate_statistics`)
   - Implement `GetMetricStatistics` (query + aggregate)

3. **Day 3: Alarm Management**
   - Implement `AlarmStore`, `AlarmRecord`, `AlarmHistoryEntry`
   - Implement `PutMetricAlarm`, `DescribeAlarms`, `DescribeAlarmsForMetric`
   - Implement `DeleteAlarms`
   - Implement `SetAlarmState` with history recording
   - Implement `EnableAlarmActions`, `DisableAlarmActions`
   - Implement `DescribeAlarmHistory`

4. **Day 4: Tags + GetMetricData + Gateway Integration**
   - Implement `TagResource`, `UntagResource`, `ListTagsForResource`
   - Implement `GetMetricData` (MetricStat queries only, no math expressions)
   - Integrate into gateway (SigV4 service name routing)
   - Update health endpoint
   - Add feature gate

5. **Day 5: Tests + Polish**
   - Unit tests for metric store, aggregation, alarm management
   - Integration tests with aws-sdk-rust
   - AWS CLI smoke tests
   - Fix edge cases

**Deliverable:** AWS CLI, Terraform `aws_cloudwatch_metric_alarm`, Grafana data source, Prometheus remote write all work.

### Phase 1: Composite Alarms + Dashboards (2-3 days)

**Goal:** PutCompositeAlarm, dashboard CRUD, insight rules. Terraform `aws_cloudwatch_dashboard` works.

6. **Day 6: Dashboards**
   - Implement `DashboardStore`, `DashboardRecord`
   - Implement `PutDashboard`, `GetDashboard`, `DeleteDashboards`, `ListDashboards`

7. **Day 7: Composite Alarms + Insight Rules**
   - Implement `PutCompositeAlarm` (store configuration, no rule evaluation)
   - Implement `PutInsightRule`, `DeleteInsightRules`, `DescribeInsightRules` (metadata storage)

**Deliverable:** Terraform `aws_cloudwatch_dashboard` and `aws_cloudwatch_composite_alarm` work.

### Phase 2: Anomaly Detection + Metric Streams (2-3 days)

**Goal:** Anomaly detector and metric stream metadata storage. Complete API surface.

8. **Day 8: Anomaly Detectors**
   - Implement `PutAnomalyDetector`, `DescribeAnomalyDetectors`, `DeleteAnomalyDetector`
   - Implement `PutManagedInsightRules`

9. **Day 9: Metric Streams**
   - Implement `PutMetricStream`, `DeleteMetricStream`, `ListMetricStreams`, `GetMetricStream`
   - Raw metrics diagnostic endpoint (`/_aws/cloudwatch/metrics/raw`)

10. **Day 10: CI + Polish**
    - CI workflow
    - Update Docker image, GitHub Action, README
    - Run LocalStack test suite subset
    - Document pass/fail

**Deliverable:** All ~31 operations implemented, CI green, Docker image updated.

---

## 15. Risk Analysis

### 15.1 Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Gateway routing conflict: CloudWatch Metrics and SNS both use awsQuery | High | High | Use SigV4 Authorization header to extract service name (`monitoring` vs `sns`). Register CloudWatch router before SNS. Fallback to Action= name inspection. |
| Aggregation math is subtly wrong for StatisticValues | Medium | High | Write comprehensive unit tests with hand-computed expected values. Test with pre-aggregated data (StatisticSets), simple values, and Values/Counts arrays separately and combined. |
| Period alignment differs from real CloudWatch | Medium | Medium | Real CloudWatch aligns periods to midnight UTC. Implement the same: `bucket_start = floor(timestamp / period) * period`. Test with timestamps that cross period boundaries. |
| Form parameter parsing fails for deeply nested structures | Medium | Medium | PutMetricData has 3 levels of nesting (`MetricData.member.1.Dimensions.member.1.Name`). Reuse and extend SNS form parser which handles similar structures. |
| awsQuery XML response format differs from real AWS | Medium | Medium | Use `quick-xml` for generation. Test response XML against what AWS SDKs expect. The SDK parses XML responses, so exact whitespace does not matter, but element names and nesting must be correct. |
| CloudWatch Logs confusion -- users may expect CW Logs and CW Metrics to be integrated | Low | Low | Document clearly that they are separate services. In the health endpoint, show `logs` and `cloudwatch` as independent services. |
| Alarm action execution (future) requires cross-service wiring | Low | Medium | For MVP, actions are stored but not executed. When implementing, follow the same trait-based abstraction as SNS-to-SQS fan-out (`SqsPublisher` pattern). Define `AlarmActionExecutor` trait in `cloudwatch-core`, implement in server binary. |
| GetMetricData with math expressions requested by users | Medium | Low | Return clear error message: "MetricDataQuery Expression is not supported. Use MetricStat instead." Document as known limitation. |
| Memory growth from unbounded metric storage | Low | Medium | Configurable `max_retention_seconds` and `max_points_per_series`. Lazy cleanup on reads. Log warnings when approaching limits. |

### 15.2 Dependencies

- `rustack-core` -- no changes needed
- `rustack-auth` -- may need `extract_sigv4_service()` utility (or implement in gateway)
- `dashmap` -- already in workspace
- `quick-xml` -- already in workspace (used by SNS)
- `serde_urlencoded` -- already in workspace (used by SNS)
- `chrono` -- for timestamp parsing and period alignment (already in workspace)

### 15.3 Decision Log

| Decision | Rationale |
|----------|-----------|
| Use `BTreeMap<i64, Vec<DataPoint>>` for time-series storage | BTreeMap provides efficient range queries by timestamp, which is critical for `GetMetricStatistics` and `GetMetricData`. Vec per timestamp handles multiple data points at the same timestamp (e.g., from concurrent PutMetricData calls). |
| Normalize dimensions by sorting | Dimensions are order-independent in AWS. Sorting by name ensures `(A=1, B=2)` and `(B=2, A=1)` resolve to the same MetricKey. This is essential for correct metric lookup. |
| SigV4 service name for gateway routing | Content-Type alone cannot distinguish CloudWatch Metrics from SNS (both awsQuery). SigV4 credential scope contains the service name, which is unambiguous and available in the request header without body parsing. |
| MVP alarm evaluation: explicit SetAlarmState only | Automatic periodic evaluation adds complexity (background task, timing, concurrency). For local dev, explicit state changes cover the testing use case. Background evaluation is a future enhancement. |
| MVP alarm actions: store but do not execute | Cross-service action execution (SNS publish, Lambda invoke) requires service wiring infrastructure. For MVP, storing actions validates IaC resource creation. Execution is a future enhancement following the SNS `SqsPublisher` trait pattern. |
| Separate crate from CloudWatch Logs | Despite sharing the "CloudWatch" brand, they are different AWS services with different protocols, types, and storage models. Separate crates follow the existing per-service pattern and avoid confusion. |
| Dashboard body as opaque JSON string | Dashboards are JSON blobs with no server-side evaluation. Storing as-is with JSON validation is sufficient. No need to parse the widget structure. |
| Phase 2 operations as metadata storage | Anomaly detection, metric streams, and insight rules involve complex evaluation logic that is not useful for local development. Accepting the API calls (for IaC compatibility) without real evaluation is the pragmatic choice. |
