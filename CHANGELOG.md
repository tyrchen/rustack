# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---
## [unreleased]

### Bug Fixes

- handle null MetricAlarms in test-action CloudWatch smoke test - ([3865234](https://github.com/commit/38652343f67b85124d460a633956384239a3c203)) - Tyr Chen
- create alarm before describe-alarms in test-action CloudWatch test - ([4866ca8](https://github.com/commit/4866ca86e3da66bc33e2c5d1ae1b921976b0d5a5)) - Tyr Chen

### Features

- add 51 Tier 1+2 operations across DynamoDB, Lambda, IAM, EventBridge (#16) - ([2ee83da](https://github.com/commit/2ee83dac83191d9b4109dd0fef7e1e69e543f195)) - Tyr Chen

### Miscellaneous Chores

- update tools - ([52ad25d](https://github.com/commit/52ad25de9ed7b2f15d1035d1c020378d4f003548)) - Tyr Chen
- upgrade actions/checkout to v6 in test-action workflow - ([279b3e7](https://github.com/commit/279b3e79e8fcc727b7f8a1778689809c74ebc80e)) - Tyr Chen

### Other

- add test-action workflow to validate GitHub Action with published GHCR image - ([34a73ef](https://github.com/commit/34a73ef43fba032c25b8878856b13de4c519b801)) - Tyr Chen

---
## [0.4.3](https://github.com/compare/v0.4.2..v0.4.3) - 2026-03-21

### Bug Fixes

- **(ci)** use separate queue for non-matching event test - ([4990c49](https://github.com/commit/4990c4947cdbfe870850e0356d86efbfff5d8dd1)) - Tyr Chen
- **(ci)** improve non-matching event test robustness - ([8d0c7ca](https://github.com/commit/8d0c7cad6aa1c79be89ace1bbb44e3369d3e177d)) - Tyr Chen
- **(codegen)** address code review findings - ([94cea62](https://github.com/commit/94cea621079f19a5c945a2c03bcaf94d5a187690)) - Tyr Chen
- **(events)** address code review findings - ([da82dec](https://github.com/commit/da82deccd0958595ca63f06b7910f2cfd792edc5)) - Tyr Chen
- **(events)** always serialize array fields in responses - ([a355bea](https://github.com/commit/a355bea2456945d3b03e6d7892e31f1388bcdd20)) - Tyr Chen
- update Dockerfile and CI to include all 18 services - ([0d81cc9](https://github.com/commit/0d81cc96dc7b61d2da7604c7ac12205d36c15892)) - Tyr Chen
- correct Docker health check service count to 18 - ([f2b57d3](https://github.com/commit/f2b57d3ca34bfc4696a9ad215bc378049f80eabe)) - Tyr Chen
- CloudWatch rpcv2Cbor routing and Kinesis test shard assignment - ([28d564e](https://github.com/commit/28d564e5e82f6bcebedb3206911020842362cdca)) - Tyr Chen
- handle epoch timestamps in CloudWatch awsJson_1.0 decode - ([4ff5e47](https://github.com/commit/4ff5e473e54eafd8b0a6fe1a0837d250f6f9ab6b)) - Tyr Chen
- handle null MetricAlarms in cloudwatch-test assertion - ([16828e0](https://github.com/commit/16828e0ca70a14a55be47b2fbb7dbba3904c81e7)) - Tyr Chen

### Features

- **(codegen)** make Smithy codegen config-driven with TOML (Phase 1) - ([b297e54](https://github.com/commit/b297e545efa3fb60ae2cefeb4b6c66a4d28dfecc)) - Tyr Chen
- **(codegen)** add protocol-aware serde generation (Phase 2) - ([edf25ae](https://github.com/commit/edf25ae997a71da65949cb64a3e0cc733fd780de)) - Tyr Chen
- **(codegen)** add error extraction and SSM/Events configs (Phase 3-4) - ([94da819](https://github.com/commit/94da819c1abffe29f1facebff4659a0a0a5ad4c6)) - Tyr Chen
- **(codegen)** add all service configs and Makefile targets (Phase 5-6) - ([7f78372](https://github.com/commit/7f7837279d70f3993e33a9d08833ecb8575069f0)) - Tyr Chen
- **(codegen)** add configs for unbuilt services, update specs - ([229019f](https://github.com/commit/229019fed11e876e8312ad4f1b817e5614f9151e)) - Tyr Chen
- **(events)** add EventBridge service - Phase 0 MVP - ([f660d75](https://github.com/commit/f660d7552b5a0b42973ac1e943ff1a471091713d)) - Tyr Chen
- **(events)** add integration tests and Makefile targets - ([a3a584c](https://github.com/commit/a3a584c1c8827d8a25dd585fb1f443fb68b2e2c3)) - Tyr Chen
- add CloudWatch Logs service (#5) - ([aba20da](https://github.com/commit/aba20da0190f6b0963ff092f574e61a563aefb86)) - Tyr Chen
- add KMS service with all 39 operations (#6) - ([88368ef](https://github.com/commit/88368ef81416db65d923b53208974121ce4e6d8c)) - Tyr Chen
- add Kinesis Data Streams service (#7) - ([4f7a895](https://github.com/commit/4f7a895ae565c347e78507ce86d442f5e00230d8)) - Tyr Chen
- add Secrets Manager service with all 23 operations (#8) - ([f87276b](https://github.com/commit/f87276b8136b0c3f9aa2c0e5e370b6f67f80619f)) - Tyr Chen
- add SES service with all 44 operations and email retrospection (#9) - ([f8983af](https://github.com/commit/f8983afe5331d46cc14d024ae268445e606073d3)) - Tyr Chen
- add API Gateway v2 service with all 57 operations and execution engine (#10) - ([35843c2](https://github.com/commit/35843c26cf3f412cc2bdea2d9963d76dcd009f9c)) - Tyr Chen
- add CloudWatch Metrics service with all 31 operations (#11) - ([6c8d12a](https://github.com/commit/6c8d12a37d4c4d21ec845d39f9d9cd33ded15580)) - Tyr Chen
- add DynamoDB Streams service with all 4 operations (#13) - ([d1c7d86](https://github.com/commit/d1c7d86343dac11c5385b5d9d06d65db4fb6a6fb)) - Tyr Chen
- add IAM service with all 60 operations (#14) - ([754767b](https://github.com/commit/754767bfc1b7016f97b8eb0f0fd204cff80ded13)) - Tyr Chen
- add STS service with all 8 operations (#15) - ([9614018](https://github.com/commit/96140180df855a7eb6fe4a7e0b88da68500c4680)) - Tyr Chen
- add awsJson_1.0 protocol support for CloudWatch - ([de120dd](https://github.com/commit/de120dd1c4416920b9ce026c243dd157d9413e01)) - Tyr Chen

### Miscellaneous Chores

- add new specs - ([56edcb0](https://github.com/commit/56edcb0d5bc634a22f1a80988052ba3d49c7b067)) - Tyr Chen

### Other

- **(events)** add EventBridge CI test workflow - ([c0d8ffc](https://github.com/commit/c0d8ffc29712f354ae0d811e9c451c6bb529f887)) - Tyr Chen
- add AWS CLI version and request debug output to cloudwatch-test - ([fd643f2](https://github.com/commit/fd643f2bda2fb8c55ef3dbae071d209f208f4341)) - Tyr Chen

---
## [0.4.2](https://github.com/compare/v0.4.1..v0.4.2) - 2026-03-10

### Bug Fixes

- **(s3)** include checksum headers in object responses (#4) - ([53f30fb](https://github.com/commit/53f30fb51d16d8f0cdc4f48f49083ecdd0a553a0)) - Tyr Chen

---
## [0.4.1](https://github.com/compare/v0.4.0..v0.4.1) - 2026-03-09

### Bug Fixes

- **(ci)** use correct check name with matrix suffix for wait-on-check - ([9a22496](https://github.com/commit/9a2249601b574186b14991b128d073112d650933)) - Tyr Chen

### Other

- gate Docker release on build workflow success - ([b2eb5c6](https://github.com/commit/b2eb5c6ed3ffc2c1fd6f75e5c21e4b93fb126b77)) - Tyr Chen

---
## [0.4.0](https://github.com/compare/v0..v0.4.0) - 2026-03-09

### Miscellaneous Chores

- bump Dockerfile base image to rust:1.93-slim - ([d2f7dd7](https://github.com/commit/d2f7dd76a9a9e5125241055519488bdf7b6df990)) - Tyr Chen

---
## [0](https://github.com/compare/v0.3.1..v0) - 2026-03-08

### Bug Fixes

- SQS FIFO long-poll timeout and group unblock notification - ([c42b527](https://github.com/commit/c42b52750ba0d83d12c4022649397b770165e6b9)) - Tyr Chen

### Features

- add GSI (Global Secondary Index) query support for DynamoDB - ([4c7b252](https://github.com/commit/4c7b252d8b748c260c0d6776e998159efdbf8e3f)) - Tyr Chen

---
## [0.3.1](https://github.com/compare/v0.3.0..v0.3.1) - 2026-03-08

### Bug Fixes

- **(health)** remove TCP half-close that breaks Docker health check (#2) - ([a08b862](https://github.com/commit/a08b86230200f7204b877d259726df48bbe173a3)) - Tyr Chen
- **(lambda)** Address code review issues from spec validation - ([c15d6db](https://github.com/commit/c15d6db678fca2f691b689fb4d189ea5a8895ed3)) - Tyr Chen
- **(lambda)** Address review issues and add boto3 compat test suite - ([e544170](https://github.com/commit/e5441707bf5aede09293447eb30d3fcef11d27ba)) - Tyr Chen
- **(sns)** Address code review issues from spec validation - ([6374395](https://github.com/commit/63743954b92a360a3176a05ae063b4538beabb04)) - Tyr Chen
- **(sns)** Add empty Result elements to void operation XML responses - ([b1d1e9f](https://github.com/commit/b1d1e9f2e6b892876c756c6a3d392219ec1da48e)) - Tyr Chen

### Features

- **(lambda)** Add Phase 0 - Lambda model, HTTP, core crates with server integration - ([6cd2e80](https://github.com/commit/6cd2e8021c915c49dbbe6871643c8a4ea44fdad8)) - Tyr Chen
- **(lambda)** Add Docker image, GitHub Action, and CI workflow for Lambda - ([a4b2b77](https://github.com/commit/a4b2b77392dc9519a5761eb4101e011b5150e94d)) - Tyr Chen
- **(sns)** Add SNS service with Phase 0 MVP operations - ([1092661](https://github.com/commit/10926615afd845cbeb40b1ae2a20c0ae6897bbc4)) - Tyr Chen
- **(sns)** Add Phase 1 - filter policies, tags, PublishBatch, ConfirmSubscription - ([e888d08](https://github.com/commit/e888d08003c367312bb9e18a1f8fd68e3cc8a849)) - Tyr Chen
- **(sns)** Add Phase 2 - FIFO topics, permissions, data protection - ([21a606a](https://github.com/commit/21a606a39f792db5238ff6bcd8b0c03eeabec321)) - Tyr Chen
- **(sns)** Add Phase 3 - platform apps, SMS, and sandbox stubs - ([68da576](https://github.com/commit/68da576a1f4ad856c7764200f44079363e098de5)) - Tyr Chen
- **(sns)** Add Docker image, GitHub Action, and CI workflow for SNS - ([c2c0f9d](https://github.com/commit/c2c0f9d7fc21518f6e520c7fd2a0f6250d802fd0)) - Tyr Chen

### Miscellaneous Chores

- add specs - ([e855084](https://github.com/commit/e8550844ed85eaa86dcbb9c44ca01e545bf5e2ff)) - Tyr Chen

### Tests

- **(lambda)** Add Lambda integration tests with aws-sdk-lambda - ([9e9b04b](https://github.com/commit/9e9b04ba601379cfc9b4549796fa95a20260950b)) - Tyr Chen
- **(sns)** Add SNS integration tests with aws-sdk-sns - ([9a31a4c](https://github.com/commit/9a31a4c0ab7e15743144cdb8d26f64925fba9dc2)) - Tyr Chen

---
## [0.3.0](https://github.com/compare/v0.2.0..v0.3.0) - 2026-03-06

### Bug Fixes

- **(ci)** Update aws-lc-sys to 0.38.0 and fix CI test assertions - ([dfce211](https://github.com/commit/dfce21107b1fd9f521eb075f24a7e77039012666)) - Tyr Chen
- **(ci)** Restore alternator conftest.py and fix SQS FIFO test flakiness - ([9c6b96f](https://github.com/commit/9c6b96f1da566586e858d485240dab3200cbc77a)) - Tyr Chen
- **(ci)** Handle empty receive response in SQS FIFO test - ([13a6bd3](https://github.com/commit/13a6bd30f34396199128050c3181dad10d24cc19)) - Tyr Chen
- **(ci)** Handle empty receive in SQS purge queue test - ([98ddb7c](https://github.com/commit/98ddb7c5cd56b298cbef1cacd0dd9bd9758f8905)) - Tyr Chen
- **(dynamodb)** Fix codex review issues and add integration tests - ([b44d38e](https://github.com/commit/b44d38e93e2c3260ed9f7139e878e7343eee9a93)) - Tyr Chen
- **(sqs)** Address 10 code review findings for correctness and AWS compatibility - ([5a934b0](https://github.com/commit/5a934b0131d1e506f53358ad5693f43cbebac4cc)) - Tyr Chen
- **(ssm)** Address 9 code review findings for correctness and AWS compatibility - ([f5ec44a](https://github.com/commit/f5ec44a4695be309290299012e5269277843b0b0)) - Tyr Chen

### Documentation

- **(specs)** Add SQS and SSM Parameter Store design specs with research - ([31d3196](https://github.com/commit/31d31965b6a0a21d3a618edc36e127ff8ab8f69f)) - Tyr Chen
- Add comparison table between RustStack and LocalStack - ([ad0663b](https://github.com/commit/ad0663b69a5ff045daca13ccf32cde0d6b34b32c)) - Tyr Chen
- Add DynamoDB API research and design spec - ([aee118c](https://github.com/commit/aee118cfa64bf1e8b8b57cae63642bb01007c001)) - Tyr Chen
- Update README and action.yml with SSM Parameter Store support - ([3e1f793](https://github.com/commit/3e1f793d9f61f8d6cfc50296ebf4d1feffb1ea83)) - Tyr Chen

### Features

- **(ci)** Add DynamoDB CI testing and update action with auth/persistence inputs - ([422153f](https://github.com/commit/422153fdff8482f9b6e1ea6267a99a847e0c4281)) - Tyr Chen
- **(ci)** Build from source in CI workflows and add SQS compat tests - ([b67cca6](https://github.com/commit/b67cca6ef1dfe3f93a8f59b82ff8fc211ef11860)) - Tyr Chen
- **(dynamodb)** Add DynamoDB crate structure and model types - ([a00859a](https://github.com/commit/a00859af632237a8c2e0b886d598355e0b6b59a2)) - Tyr Chen
- **(dynamodb)** Add ruststack-dynamodb-http crate - ([d93bef7](https://github.com/commit/d93bef76b9a745af200e603fffdc6d4018be177d)) - Tyr Chen
- **(dynamodb)** Add ruststack-dynamodb-core crate with business logic - ([4931b9a](https://github.com/commit/4931b9a886348e24bea99c34d23b858f382633fb)) - Tyr Chen
- **(dynamodb)** Integrate DynamoDB into unified gateway server - ([4d1e0eb](https://github.com/commit/4d1e0eb53cb3ac7676cb79fffc56407afc5e4978)) - Tyr Chen
- **(dynamodb)** Fix 181 Alternator compatibility test failures - ([855c12f](https://github.com/commit/855c12f2a4e3f0dd1344566f9eff8b167f8fad72)) - Tyr Chen
- **(dynamodb)** Fix 245+ Alternator test failures with legacy API, expressions, and validation - ([d4cef2a](https://github.com/commit/d4cef2a308bf7ed8639cba508527f320c87560a7)) - Tyr Chen
- **(dynamodb)** Fix all remaining Alternator test failures (457/457 pass) - ([6c10945](https://github.com/commit/6c109451ed8a33e5d299c1934f824047e73c86ff)) - Tyr Chen
- **(server)** Add selective service enablement with ServiceRouter trait - ([849827c](https://github.com/commit/849827ce43a35038d327f2e40c4472ac68223ba4)) - Tyr Chen
- **(sqs)** Add SQS service with all 23 operations (Phase 0) - ([a5c75d6](https://github.com/commit/a5c75d6618252ad2b4a84c34b464d73cbbf8cd49)) - Tyr Chen
- **(sqs)** Add FIFO queue support with deduplication and message groups - ([ef779aa](https://github.com/commit/ef779aa54bb117d88e872c040cf49d5323474a62)) - Tyr Chen
- **(sqs)** Add integration tests, Docker image, and GitHub Action support for SQS - ([0fbf5b4](https://github.com/commit/0fbf5b4a023236f43854eb22b7f03f5755c1d34d)) - Tyr Chen
- **(ssm)** Add SSM Parameter Store with 6 core operations (Phase 0) - ([3e868bc](https://github.com/commit/3e868bc1aefbb50088c9cfd89f8ce0a75305e650)) - Tyr Chen
- **(ssm)** Add metadata, history, and tag operations (Phase 1) - ([7a6e5af](https://github.com/commit/7a6e5af6a441622f8b8ef6903d53929ee7432f79)) - Tyr Chen
- **(ssm)** Add label management operations (Phase 2) - ([486ba89](https://github.com/commit/486ba89c656216101ba93eba9a88de4647147e15)) - Tyr Chen
- **(ssm)** Add Docker image, GitHub Action, and CI workflow for SSM - ([3bb519e](https://github.com/commit/3bb519edd7b0db08dd7fec4f1e68ad5f04b64f0f)) - Tyr Chen

### Tests

- **(dynamodb)** Add Alternator compatibility test suite and fix response serialization - ([f3a1ea1](https://github.com/commit/f3a1ea12cd806b040e8bd2c7904a98257a0bbf23)) - Tyr Chen
- **(ssm)** Add 19 integration tests for SSM Parameter Store - ([cae1ff6](https://github.com/commit/cae1ff6f8defb7a2ba7bde3755154542093dd2b5)) - Tyr Chen

---
## [0.2.0](https://github.com/compare/v0.1.0..v0.2.0) - 2026-03-01

### Bug Fixes

- **(ci)** Fix s3-test large object and cleanup step failures - ([135b8a4](https://github.com/commit/135b8a49391721a7dea301c53a9f8444684ea1c9)) - Tyr Chen
- **(ci)** Use 1MB file in s3-test to avoid server temp file spooling issue - ([6042aff](https://github.com/commit/6042aff0154a57ee030572df2f6223388cbddbaf)) - Tyr Chen
- **(ci)** Use put-object instead of s3 cp to avoid chunked encoding - ([d85aa67](https://github.com/commit/d85aa67a0186bf4caaf0259917a105ada5da9d5b)) - Tyr Chen
- **(ci)** Mark large object tests as continue-on-error - ([19422a0](https://github.com/commit/19422a00f1a3f971b11038309b92d4ce99e30fd8)) - Tyr Chen
- **(ci)** Mark bucket config step as continue-on-error - ([7069845](https://github.com/commit/7069845c5e3e452811f9f2440ad2015644f8154c)) - Tyr Chen
- **(ci)** Use temp file instead of /dev/stdin for listing test objects - ([533ee1a](https://github.com/commit/533ee1a40a2408cfd258a81062825d33f1a86cf0)) - Tyr Chen
- **(ci)** Show put-object output in listing step for debugging - ([d468395](https://github.com/commit/d4683952d9e561477bdfa4fa75dc4383dfb6dff2)) - Tyr Chen
- **(ci)** Use jq to count objects instead of KeyCount in listing test - ([0a4f05d](https://github.com/commit/0a4f05d8c5066ebffb4721e47b6d6552792ace4a)) - Tyr Chen
- Add /tmp to Docker scratch image for temp file spillover - ([67d2ae0](https://github.com/commit/67d2ae0aae24839a9cda201ade81ebd8770be1a4)) - Tyr Chen

### Documentation

- Add GitHub Action usage guide to README - ([e3bfe1d](https://github.com/commit/e3bfe1d59a8f1108984e76d3d2f3303ba7efd55f)) - Tyr Chen

### Other

- Add comprehensive S3 test workflow using RustStack GitHub Action - ([0abc2d4](https://github.com/commit/0abc2d4d026146fc827feed068f9993642bef263)) - Tyr Chen
- Add workflow_dispatch to release-docker for manual image builds - ([b3ab88b](https://github.com/commit/b3ab88b87516629c0b77082d8e9a6b53f1ff8dee)) - Tyr Chen

### Tests

- Add CORS configuration XML deserialization test - ([89e5d02](https://github.com/commit/89e5d022762a66100650761ce1b6a80f5d0f6f8d)) - Tyr Chen

---
## [0.1.0] - 2026-03-01

### Bug Fixes

- Phase 5 code review - Fix HTTP status codes, dedup parse_copy_source, typed notification config - ([a8506a2](https://github.com/commit/a8506a23059a26d74d6381d198b8eaeb8d1d25fd)) - Tyr Chen
- Fix 6 Mint test root causes and CI masking - ([0ed4205](https://github.com/commit/0ed42052b0ae0d5244e21eecbd6477ca3ccb2d54)) - Tyr Chen
- Use pass-count threshold for Mint CI instead of fail-count - ([7d7dc28](https://github.com/commit/7d7dc28177a56c8e24ea7c73864f9a6909f70917)) - Tyr Chen
- Fix remaining Mint test failures (Content-MD5, lifecycle, metrics, auth) - ([544568b](https://github.com/commit/544568b896298dcaf73a543b3f91514b17a7a356)) - Tyr Chen
- Validate X-Amz-Content-Sha256 header and raise CI threshold to 7 - ([87a82c3](https://github.com/commit/87a82c3e9a3ba7cef474876757b157e99f47a94f)) - Tyr Chen
- Fix 4 more Mint test issues, add local Mint testing (5/15 passing) - ([370cc5c](https://github.com/commit/370cc5cfa53b7cce284ffa90fe3f34cc3592c419)) - Tyr Chen
- Enforce Object Lock on version deletes and decode aws-chunked bodies - ([11475b6](https://github.com/commit/11475b6ee22de75dd648ce83de85810941c87f65)) - Tyr Chen
- Use 5MB part size in multipart integration test - ([eb29525](https://github.com/commit/eb29525b32b2ea7a9d8aa2a5729db583088fd9cd)) - Tyr Chen
- Update Dockerfile for all workspace members - ([c57dec0](https://github.com/commit/c57dec07bc953af5a69397fc92d62249e504974f)) - Tyr Chen
- Copy rust-toolchain.toml before adding musl targets in Dockerfile - ([9891695](https://github.com/commit/9891695451c89243f9d5bfdf66188811e5b4a7c1)) - Tyr Chen

### Features

- implement S3 service with workspace restructure - ([3308e53](https://github.com/commit/3308e53b38678608fc7d1bbb5cf855b91ed9903e)) - Tyr Chen
- add Docker image, GitHub Action, and CI/CD workflows - ([adeb3b2](https://github.com/commit/adeb3b24cb0ae1d70917bf517381057294f20be5)) - Tyr Chen
- add integration test suite and CI workflows - ([b2b5286](https://github.com/commit/b2b5286accd1279a71f84f234c1f677f822b10af)) - Tyr Chen
- Phase 1 - Smithy S3 model codegen, types, and SigV4 auth - ([37588a5](https://github.com/commit/37588a52cf1ccaadbb28b5a362e3e255839e51fb)) - Tyr Chen
- Phase 2 - S3 XML serialization, HTTP routing, and service layer - ([dd5c3bf](https://github.com/commit/dd5c3bf480c43096ed51c8fb1973ccc326121e4c)) - Tyr Chen
- Phase 3 - Remove s3s dependency, use own S3 model types - ([3cc7582](https://github.com/commit/3cc758233ab47d843b07f706e5a29a3873c07f8d)) - Tyr Chen
- Phase 4 - Fix XML serialization/deserialization and versioning bugs - ([2e1a970](https://github.com/commit/2e1a970589d08a0e41cba957546c94bfdbeb57b2)) - Tyr Chen
- Add SigV2 auth, rename ruststack-s3-auth to ruststack-auth, fix Mint tests (7/15) - ([8192e01](https://github.com/commit/8192e016e31e5a8521e704e1851b8ba9a7ab1811)) - Tyr Chen
- Add PostObject, BypassGovernanceRetention, fix SigV4 auth (8/15 Mint) - ([6c10853](https://github.com/commit/6c108535a0683141318375dc4dffe982d5607069)) - Tyr Chen

### Miscellaneous Chores

- init project - ([2c4d76a](https://github.com/commit/2c4d76ab78785c71dcc19427ad516bc2b9f3f083)) - Tyr Chen
- rename project from localstack-rs to ruststack - ([eddbfb0](https://github.com/commit/eddbfb0b63542b14a47b2b1bb9f01d34d9edb48c)) - Tyr Chen

<!-- generated by git-cliff -->
