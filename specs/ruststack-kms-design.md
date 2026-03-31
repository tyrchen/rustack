# Rustack KMS: Native Rust Implementation Design

**Date:** 2026-03-06
**Status:** Draft / RFC
**Depends on:** [smithy-s3-redesign-design.md](./smithy-s3-redesign-design.md), [rustack-dynamodb-design.md](./rustack-dynamodb-design.md), [rustack-ssm-design.md](./rustack-ssm-design.md)
**Scope:** Add KMS support to Rustack -- ~35 operations covering key management, symmetric/asymmetric encryption, signing, HMAC, grants, aliases, and envelope encryption, using the same Smithy-based codegen and gateway routing patterns established by DynamoDB and SSM.

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Motivation](#2-motivation)
3. [Goals and Non-Goals](#3-goals-and-non-goals)
4. [Architecture Overview](#4-architecture-overview)
5. [Protocol Design: awsJson1.1](#5-protocol-design-awsjson11)
6. [Smithy Code Generation Strategy](#6-smithy-code-generation-strategy)
7. [Crate Structure](#7-crate-structure)
8. [HTTP Layer Design](#8-http-layer-design)
9. [Cryptographic Engine Design](#9-cryptographic-engine-design)
10. [Storage Engine Design](#10-storage-engine-design)
11. [Core Business Logic](#11-core-business-logic)
12. [Error Handling](#12-error-handling)
13. [Server Integration](#13-server-integration)
14. [Testing Strategy](#14-testing-strategy)
15. [Phased Implementation Plan](#15-phased-implementation-plan)
16. [Risk Analysis](#16-risk-analysis)

---

## 1. Executive Summary

This spec proposes adding KMS (Key Management Service) support to Rustack as a fully native Rust implementation. Key design decisions:

- **Medium scope** -- ~35 operations organized into key management, cryptographic operations, alias management, grant management, and key policy operations. More complex than SSM (13 ops) but simpler than DynamoDB (66 ops) because the protocol is identical (`awsJson1.1`) and the complexity lies in the cryptographic engine rather than storage.
- **Real cryptography** -- unlike SSM where SecureString is plaintext, KMS must perform real AES-256-GCM encryption, RSA encryption/signing, ECDSA signing, and HMAC operations. We use the `aws-lc-rs` crate (AWS's own Rust crypto library, API-compatible with `ring`) for all cryptographic operations.
- **Custom ciphertext blob format** -- for symmetric encryption, we use a self-describing binary format that embeds the key ID, IV, and authentication tag in the ciphertext blob header. This allows `Decrypt` to identify the correct key without requiring the caller to specify `KeyId`. The format is compatible with what AWS SDKs expect (opaque binary blob, base64-encoded over the wire).
- **Envelope encryption support** -- `GenerateDataKey` and `GenerateDataKeyWithoutPlaintext` are critical operations that enable S3 SSE-KMS, Secrets Manager encryption, and most AWS service integrations. These are prioritized in Phase 0.
- **Key state machine** -- full implementation of key states (Enabled, Disabled, PendingDeletion, PendingImport) with proper state transition enforcement.
- **Estimated effort** -- 5-7 days for Phase 0 MVP (key management + symmetric encryption + aliases), 10-14 days for full implementation including asymmetric keys, signing, HMAC, and grants.

---

## 2. Motivation

### 2.1 Why KMS?

KMS is the foundational security primitive in AWS. Nearly every other AWS service depends on it for encryption:

- **S3 SSE-KMS** -- server-side encryption with customer-managed keys. S3 calls `GenerateDataKey` for each object upload and `Decrypt` for each download. Without KMS, `aws s3 cp --sse aws:kms` fails.
- **Secrets Manager** -- encrypts all secrets with KMS. `aws secretsmanager create-secret` requires a working KMS for encryption.
- **DynamoDB encryption at rest** -- uses KMS-managed keys. While our in-memory DynamoDB does not persist, Terraform plans that enable encryption need `DescribeKey` to validate the key ARN.
- **Terraform and CDK** -- `aws_kms_key` and `aws_kms_alias` are among the most common Terraform resources. Without KMS, infrastructure-as-code workflows that reference KMS keys fail during `terraform plan`.
- **Application envelope encryption** -- applications that use the AWS Encryption SDK or implement their own envelope encryption call `GenerateDataKey` to get a data key, encrypt locally, and store the ciphertext blob alongside the encrypted data.
- **SSM SecureString** -- once KMS is available, SSM SecureString parameters can actually be encrypted/decrypted rather than stored as plaintext.
- **Lambda, ECS, EKS** -- environment variable encryption, secrets injection, and volume encryption all reference KMS keys.

### 2.2 Complexity Assessment

| Dimension | KMS | SSM | DynamoDB | S3 |
|-----------|-----|-----|----------|-----|
| Total operations | ~35 | 13 | 66 | 90+ |
| Protocol | awsJson1.1 (reuse) | awsJson1.1 (reuse) | awsJson1.0 | RestXml |
| Complex parsers needed | 0 | 0 | 1 (expressions) | 1 (XML) |
| Storage engine complexity | DashMap + key state machine | HashMap + BTreeMap | B-Tree + GSI/LSI | Object store + multipart |
| Crypto requirements | AES-GCM, RSA, ECC, HMAC | None | None | Checksum only |
| Concurrency model | Request/response | Request/response | Transactions, batch | Multipart upload, streaming |
| Estimated lines of code | ~6,000-8,000 | ~3,000 | ~15,000 | ~12,700 |

The primary complexity in KMS is the cryptographic engine, not the storage or protocol. The protocol layer (`awsJson1.1`) is shared with SSM. The storage model is a flat `DashMap` of keys with associated metadata.

### 2.3 Tool Coverage

With the full KMS implementation, the following tools and integrations work:

| Tool / Integration | Operations Used | Phase Available |
|--------------------|----------------|-----------------|
| AWS CLI (`aws kms`) | All operations | Phase 0+ |
| Terraform `aws_kms_key` / `aws_kms_alias` | CreateKey, DescribeKey, CreateAlias, ListAliases, TagResource | Phase 0 |
| S3 SSE-KMS (`--sse aws:kms`) | GenerateDataKey, Decrypt | Phase 0 |
| AWS Encryption SDK | GenerateDataKey, Decrypt, Encrypt | Phase 0 |
| Secrets Manager | Encrypt, Decrypt, GenerateDataKey | Phase 0 |
| CDK KMS constructs | CreateKey, DescribeKey, CreateAlias, PutKeyPolicy | Phase 0 |
| Terraform grants | CreateGrant, ListGrants, RevokeGrant | Phase 2 |
| Asymmetric signing (JWT, etc.) | CreateKey (RSA/ECC), Sign, Verify, GetPublicKey | Phase 1 |
| HMAC-based auth | CreateKey (HMAC), GenerateMac, VerifyMac | Phase 1 |
| Key rotation workflows | EnableKeyRotation, GetKeyRotationStatus | Phase 2 |

---

## 3. Goals and Non-Goals

### 3.1 Goals

1. **Core key management** -- CreateKey, DescribeKey, ListKeys, EnableKey, DisableKey, ScheduleKeyDeletion, CancelKeyDeletion, UpdateKeyDescription
2. **Symmetric encryption** -- Encrypt, Decrypt, ReEncrypt with AES-256-GCM (SYMMETRIC_DEFAULT)
3. **Envelope encryption** -- GenerateDataKey, GenerateDataKeyWithoutPlaintext, GenerateDataKeyPair, GenerateDataKeyPairWithoutPlaintext
4. **Asymmetric encryption** -- RSA OAEP encryption/decryption (RSA_2048, RSA_3072, RSA_4096)
5. **Digital signatures** -- Sign, Verify with RSA (PSS, PKCS1v1.5) and ECDSA (P-256, P-384, P-521, secp256k1)
6. **HMAC** -- GenerateMac, VerifyMac with HMAC-SHA-224/256/384/512
7. **Alias management** -- CreateAlias, DeleteAlias, ListAliases, UpdateAlias
8. **Tag management** -- TagResource, UntagResource, ListResourceTags
9. **Key policies** -- GetKeyPolicy, PutKeyPolicy, ListKeyPolicies (store but do not enforce)
10. **Grants** -- CreateGrant, ListGrants, RetireGrant, RevokeGrant, ListRetirableGrants (store but do not enforce permissions)
11. **Key rotation** -- EnableKeyRotation, DisableKeyRotation, GetKeyRotationStatus
12. **Random generation** -- GenerateRandom
13. **Public key retrieval** -- GetPublicKey for asymmetric keys
14. **Smithy-generated types** -- all KMS types generated from official AWS Smithy model
15. **Shared infrastructure** -- reuse `rustack-core`, `rustack-auth`, and the `awsJson1.1` protocol layer from SSM
16. **Same Docker image** -- single binary serves S3 + DynamoDB + SQS + SSM + KMS on port 4566
17. **Key state machine** -- full state transitions (Enabled, Disabled, PendingDeletion, PendingImport)
18. **Self-describing ciphertext** -- ciphertext blobs embed key ID, enabling `Decrypt` without explicit `KeyId`
19. **Encryption context** -- support encryption context as AAD (additional authenticated data)

### 3.2 Non-Goals

1. **Multi-region keys** -- `ReplicateKey` and multi-region configuration are not implemented in MVP
2. **Custom key stores** -- CloudHSM and external key store integration
3. **Key material import** -- `GetParametersForImport`, `ImportKeyMaterial`, `DeleteImportedKeyMaterial` (complex wrapping key management)
4. **IAM policy enforcement** -- key policies are stored but not evaluated for authorization decisions
5. **Grant constraint enforcement** -- grants are stored and listed but do not affect authorization
6. **Automatic key rotation execution** -- rotation status is tracked but keys are not actually rotated on schedule
7. **On-demand key rotation** -- `RotateKeyOnDemand` (complex key material versioning)
8. **CloudTrail integration** -- no audit logging
9. **Rate limiting** -- no throttling of API requests
10. **Data persistence across restarts** -- in-memory only, matching behavior of other Rustack services
11. **Nitro Enclave attestation** -- `Recipient` parameter for `Decrypt` (requires PKCS7 envelope construction)
12. **DeriveSharedSecret** -- ECDH key agreement (can be added later)
13. **Post-quantum key types** -- ML-KEM and other PQ algorithms

---

## 4. Architecture Overview

### 4.1 Layered Architecture

```
                    AWS SDK / CLI / Terraform
                         |
                         | HTTP POST :4566
                         v
              +---------------------+
              |   Gateway Router    |  X-Amz-Target dispatch
              +--------+------------+
                       |
         +------+------+------+------+------+
         |      |      |      |      |
         v      v      v      v      v
   +------+ +------+ +------+ +------+ +------+
   | S3   | | DDB  | | SQS  | | SSM  | | KMS  |
   | HTTP | | HTTP | | HTTP | | HTTP | | HTTP |
   +--+---+ +--+---+ +--+---+ +--+---+ +--+---+
      |        |        |        |        |
   +--+---+ +--+---+ +--+---+ +--+---+ +--+---+
   | S3   | | DDB  | | SQS  | | SSM  | | KMS  |
   | Core | | Core | | Core | | Core | | Core |
   +--+---+ +--+---+ +--+---+ +--+---+ +--+---+
      |        |        |        |        |
      +--------+--------+--------+--------+
                       |
              +--------+--------+
              | rustack-core  |
              | rustack-auth  |
              +-----------------+
```

### 4.2 Gateway Routing

KMS requests are distinguished by the `X-Amz-Target` header prefix `TrentService.`:

| Service | X-Amz-Target Prefix | Content-Type |
|---------|---------------------|--------------|
| DynamoDB | `DynamoDB_20120810.` | `application/x-amz-json-1.0` |
| SQS | `AmazonSQS.` | `application/x-amz-json-1.0` |
| SSM | `AmazonSSM.` | `application/x-amz-json-1.1` |
| KMS | `TrentService.` | `application/x-amz-json-1.1` |
| S3 | *(absent)* | varies |

Routing logic: check `X-Amz-Target` header. If prefix is `TrentService.`, route to KMS. The prefix "TrentService" is AWS's internal service name for KMS (named after the Trent protocol in cryptographic key management).

### 4.3 Crate Dependency Graph

```
rustack (app)
+-- rustack-core
+-- rustack-auth
+-- rustack-s3-{model,core,http}
+-- rustack-dynamodb-{model,core,http}
+-- rustack-sqs-{model,core,http}
+-- rustack-ssm-{model,core,http}
+-- rustack-kms-model        <-- NEW (auto-generated)
+-- rustack-kms-core         <-- NEW
+-- rustack-kms-http          <-- NEW

rustack-kms-http
+-- rustack-kms-model
+-- rustack-auth

rustack-kms-core
+-- rustack-core
+-- rustack-kms-model
+-- aws-lc-rs                  <-- NEW dependency (crypto)

rustack-kms-model (auto-generated, standalone)
```

---

## 5. Protocol Design: awsJson1.1

### 5.1 Protocol Details

KMS uses `awsJson1.1`, identical to SSM. The entire JSON serialization, routing, and error formatting infrastructure from SSM can be reused.

| Aspect | SSM (awsJson1.1) | KMS (awsJson1.1) |
|--------|-------------------|-------------------|
| HTTP Method | POST only | POST only |
| URL Path | `/` always | `/` always |
| Content-Type | `application/x-amz-json-1.1` | `application/x-amz-json-1.1` |
| X-Amz-Target | `AmazonSSM.<Op>` | `TrentService.<Op>` |
| Request body | JSON | JSON |
| Response body | JSON | JSON |
| Error `__type` | Short name | Short name |
| Timestamp format | Epoch seconds (double) | Epoch seconds (double) |
| Auth | SigV4, service=`ssm` | SigV4, service=`kms` |

### 5.2 Binary Data Encoding

KMS operations frequently pass binary data (ciphertext blobs, plaintext, signatures, public keys). In the `awsJson1.1` protocol, binary data (`blob` type in Smithy) is base64-encoded in the JSON request/response bodies. The AWS SDKs handle this encoding/decoding transparently.

In the Smithy-generated model types, blob fields will be represented as `Vec<u8>` with custom serde serializers/deserializers that handle the base64 encoding:

```rust
/// Custom serde for base64-encoded blobs in JSON.
mod base64_blob {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        STANDARD.encode(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        STANDARD.decode(&s).map_err(serde::de::Error::custom)
    }
}
```

### 5.3 What We Reuse

| Component | Reusable? | Notes |
|-----------|-----------|-------|
| JSON request deserialization | Yes | `serde_json::from_slice` with generated `Deserialize` derives |
| JSON response serialization | Yes | `serde_json::to_vec` with generated `Serialize` derives |
| `X-Amz-Target` header parsing | Yes | Same pattern, different prefix (`TrentService.`) |
| JSON error formatting | Yes | Same `{"__type": "...", "message": "..."}` format |
| SigV4 auth | Yes | `rustack-auth` is service-agnostic |
| Multi-account/region state | Yes | `rustack-core` unchanged |
| Base64 blob serde | Partial | SSM does not use blob types; add to codegen for KMS |

---

## 6. Smithy Code Generation Strategy

### 6.1 Universal Codegen

The `rustack-kms-model` crate is generated from the official AWS Smithy JSON AST using the universal codegen tool at `codegen/`. The codegen reads a TOML service configuration and the Smithy model to produce all model types with correct serde attributes.

**Smithy model:** `codegen/smithy-model/kms.json` (764KB, namespace `com.amazonaws.kms`, 39 operations)
**Service config:** `codegen/services/kms.toml`
**Generate:** `make codegen-kms`

### 6.2 Generated Output

The codegen produces 6 files in `crates/rustack-kms-model/src/`:

| File | Contents |
|------|----------|
| `lib.rs` | Module declarations and re-exports |
| `types.rs` | Shared types (enums and structs) with serde derives |
| `operations.rs` | `KmsOperation` enum with `as_str()`, `from_name()`, phase methods |
| `error.rs` | `KmsErrorCode` enum + `KmsError` struct + `kms_error!` macro |
| `input.rs` | All input structs with `#[serde(rename_all = "PascalCase")]` |
| `output.rs` | All output structs with serde derives |

### 6.3 Service-Specific Notes

KMS uses blob types extensively (binary ciphertext/plaintext). The codegen generates these as `bytes::Bytes`. Custom base64 serde handling may need an overlay file if serde is used for direct JSON serialization of blob fields.

See [smithy-codegen-all-services-design.md](./smithy-codegen-all-services-design.md) for full codegen architecture details.

---

## 7. Crate Structure

### 7.1 `rustack-kms-model` (auto-generated)

```
crates/rustack-kms-model/
+-- Cargo.toml
+-- src/
    +-- lib.rs              # Module re-exports
    +-- types.rs            # Auto-generated: enums + shared structs
    +-- operations.rs       # Auto-generated: KmsOperation enum
    +-- error.rs            # Auto-generated: KmsError + error codes
    +-- input.rs            # Auto-generated: all ~35 input structs
    +-- output.rs           # Auto-generated: all ~35 output structs
    +-- blob.rs             # Base64 blob serde helpers
```

**Dependencies:** `serde`, `serde_json`, `base64`

### 7.2 `rustack-kms-core`

```
crates/rustack-kms-core/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- config.rs           # KmsConfig
    +-- provider.rs         # RustackKms (main provider, all operation handlers)
    +-- error.rs            # KmsServiceError
    +-- state.rs            # KmsStore (DashMap-based key/alias/grant store)
    +-- key.rs              # KmsKey, key state machine, key metadata
    +-- crypto.rs           # CryptoEngine (AES-GCM, RSA, ECC, HMAC)
    +-- ciphertext.rs       # Ciphertext blob serialization/deserialization
    +-- alias.rs            # Alias management (DashMap<String, String>)
    +-- grant.rs            # Grant storage and lookup
    +-- policy.rs           # Key policy storage
    +-- validation.rs       # Input validation (key ID formats, tag limits, etc.)
    +-- resolve.rs          # Key ID resolution (UUID, ARN, alias name, alias ARN)
```

**Dependencies:** `rustack-core`, `rustack-kms-model`, `aws-lc-rs`, `dashmap`, `serde_json`, `chrono`, `uuid`, `tracing`, `base64`

### 7.3 `rustack-kms-http`

```
crates/rustack-kms-http/
+-- Cargo.toml
+-- src/
    +-- lib.rs
    +-- router.rs           # TrentService.* target dispatch
    +-- service.rs          # KmsHttpService (hyper Service impl)
    +-- dispatch.rs         # KmsHandler trait + operation dispatch
```

**Dependencies:** `rustack-kms-model`, `rustack-auth`, `hyper`, `serde_json`, `bytes`

This crate is structurally identical to `rustack-ssm-http`. The router parses `TrentService.<Op>` instead of `AmazonSSM.<Op>`.

### 7.4 Workspace Changes

```toml
[workspace.dependencies]
rustack-kms-model = { path = "crates/rustack-kms-model" }
rustack-kms-http = { path = "crates/rustack-kms-http" }
rustack-kms-core = { path = "crates/rustack-kms-core" }

# New crypto dependency
aws-lc-rs = "1"

# Testing
aws-sdk-kms = "1"
```

---

## 8. HTTP Layer Design

### 8.1 Router

```rust
/// KMS operation router.
///
/// Parses the `X-Amz-Target: TrentService.<Op>` header to determine the operation.
pub struct KmsRouter;

impl KmsRouter {
    pub fn resolve(target: &str) -> Result<KmsOperation, KmsError> {
        let op_name = target
            .strip_prefix("TrentService.")
            .ok_or_else(|| KmsError::unknown_operation(target))?;

        KmsOperation::from_name(op_name)
            .ok_or_else(|| KmsError::unknown_operation(op_name))
    }
}
```

### 8.2 ServiceRouter Trait Implementation

```rust
/// KMS service router for the gateway.
pub struct KmsServiceRouter<H: KmsHandler> {
    inner: KmsHttpService<H>,
}

impl<H: KmsHandler> ServiceRouter for KmsServiceRouter<H> {
    fn name(&self) -> &'static str {
        "kms"
    }

    fn matches(&self, req: &Request<()>) -> bool {
        req.headers()
            .get("x-amz-target")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| s.starts_with("TrentService."))
    }

    async fn call(&self, req: Request<Incoming>) -> Response<GatewayBody> {
        // 1. Extract X-Amz-Target, resolve to KmsOperation
        // 2. Read body bytes
        // 3. Deserialize JSON input
        // 4. Extract account/region from auth headers
        // 5. Dispatch to handler
        // 6. Serialize JSON output or error
    }
}
```

### 8.3 KmsHandler Trait

```rust
/// Trait defining all KMS operations.
///
/// Implemented by `RustackKms` in the core crate.
#[async_trait]
pub trait KmsHandler: Send + Sync + 'static {
    async fn create_key(
        &self,
        account_id: &str,
        region: &str,
        input: CreateKeyInput,
    ) -> Result<CreateKeyOutput, KmsServiceError>;

    async fn encrypt(
        &self,
        account_id: &str,
        region: &str,
        input: EncryptInput,
    ) -> Result<EncryptOutput, KmsServiceError>;

    async fn decrypt(
        &self,
        account_id: &str,
        region: &str,
        input: DecryptInput,
    ) -> Result<DecryptOutput, KmsServiceError>;

    // ... one method per operation
}
```

---

## 9. Cryptographic Engine Design

### 9.1 Overview

The cryptographic engine is the most complex component of the KMS implementation. It must:

1. Generate and store key material for all supported key types
2. Perform real encryption/decryption that produces valid ciphertext
3. Perform real digital signatures that external verifiers can validate
4. Produce ciphertext blobs that AWS SDKs can handle (opaque binary, base64-encoded)

### 9.2 Crypto Library Choice: `aws-lc-rs`

We use `aws-lc-rs` (AWS LibCrypto for Rust) for all cryptographic operations. Rationale:

| Criterion | `aws-lc-rs` | `ring` | `RustCrypto` |
|-----------|-------------|--------|--------------|
| AES-256-GCM | Yes | Yes | Yes |
| RSA keygen | Yes | No (verify only) | Yes |
| RSA OAEP encrypt/decrypt | Yes | No | Yes |
| RSA PSS sign/verify | Yes | Yes | Yes |
| ECDSA P-256/P-384/P-521 | Yes | P-256/P-384 only | Yes |
| ECDSA secp256k1 | No | No | Yes (k256) |
| HMAC-SHA-224/256/384/512 | Yes | Yes | Yes |
| FIPS compliance | Yes (aws-lc-fips-sys) | No | No |
| Maintained by | AWS | Brian Smith | Community |
| API compatibility | ring-compatible | N/A | Different |

`aws-lc-rs` is the best fit because it supports all required algorithms including RSA key generation and OAEP encryption, which `ring` lacks. For `secp256k1` (used by `ECC_SECG_P256K1` key spec), we fall back to the `k256` crate from RustCrypto.

### 9.3 Key Material Types

```rust
/// Cryptographic key material for a KMS key.
///
/// Stores the raw key bytes and derived objects needed for crypto operations.
/// Key material never leaves the process -- only ciphertext and public keys
/// are exposed via the API.
pub enum CryptoKeyMaterial {
    /// AES-256 symmetric key (32 bytes).
    /// Used for SYMMETRIC_DEFAULT key spec.
    Symmetric {
        /// Raw 256-bit key.
        key_material: Vec<u8>,
    },

    /// RSA key pair.
    /// Used for RSA_2048, RSA_3072, RSA_4096 key specs.
    Rsa {
        /// PKCS#8 DER-encoded private key.
        private_key_der: Vec<u8>,
        /// SubjectPublicKeyInfo DER-encoded public key.
        public_key_der: Vec<u8>,
    },

    /// Elliptic curve key pair.
    /// Used for ECC_NIST_P256, ECC_NIST_P384, ECC_NIST_P521, ECC_SECG_P256K1.
    Ecc {
        /// PKCS#8 DER-encoded private key.
        private_key_der: Vec<u8>,
        /// SubjectPublicKeyInfo DER-encoded public key.
        public_key_der: Vec<u8>,
        /// The curve used.
        curve: EccCurve,
    },

    /// HMAC key material.
    /// Used for HMAC_224, HMAC_256, HMAC_384, HMAC_512 key specs.
    Hmac {
        /// Raw key bytes. Length depends on key spec
        /// (28-64 bytes for HMAC_224, 32-64 for HMAC_256, etc.).
        key_material: Vec<u8>,
    },
}
```

### 9.4 Key Generation

```rust
/// Generate fresh key material for the given key spec.
pub fn generate_key_material(key_spec: &KeySpec) -> Result<CryptoKeyMaterial> {
    match key_spec {
        KeySpec::SymmetricDefault => {
            let mut key = vec![0u8; 32];
            aws_lc_rs::rand::fill(&mut key)?;
            Ok(CryptoKeyMaterial::Symmetric { key_material: key })
        }
        KeySpec::Rsa2048 | KeySpec::Rsa3072 | KeySpec::Rsa4096 => {
            let key_size = match key_spec {
                KeySpec::Rsa2048 => 2048,
                KeySpec::Rsa3072 => 3072,
                KeySpec::Rsa4096 => 4096,
                _ => unreachable!(),
            };
            let private_key = aws_lc_rs::rsa::KeyPair::generate(key_size)?;
            let private_key_der = private_key.as_der().as_ref().to_vec();
            let public_key_der = private_key
                .public_key()
                .as_ref()
                .to_vec();
            Ok(CryptoKeyMaterial::Rsa {
                private_key_der,
                public_key_der,
            })
        }
        KeySpec::EccNistP256 | KeySpec::EccNistP384 | KeySpec::EccNistP521 => {
            let algorithm = match key_spec {
                KeySpec::EccNistP256 => &aws_lc_rs::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
                KeySpec::EccNistP384 => &aws_lc_rs::signature::ECDSA_P384_SHA384_ASN1_SIGNING,
                // P-521 may need special handling
                _ => return Err(anyhow!("Unsupported ECC curve for generation")),
            };
            let rng = aws_lc_rs::rand::SystemRandom::new();
            let pkcs8 = aws_lc_rs::signature::EcdsaKeyPair::generate_pkcs8(algorithm, &rng)?;
            let key_pair = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
                algorithm,
                pkcs8.as_ref(),
            )?;
            let public_key_der = key_pair.public_key().as_ref().to_vec();
            Ok(CryptoKeyMaterial::Ecc {
                private_key_der: pkcs8.as_ref().to_vec(),
                public_key_der,
                curve: key_spec_to_curve(key_spec),
            })
        }
        KeySpec::EccSecgP256k1 => {
            // Use k256 crate for secp256k1
            generate_secp256k1_key()
        }
        KeySpec::Hmac224 | KeySpec::Hmac256 | KeySpec::Hmac384 | KeySpec::Hmac512 => {
            let (min_len, max_len) = match key_spec {
                KeySpec::Hmac224 => (28, 64),
                KeySpec::Hmac256 => (32, 64),
                KeySpec::Hmac384 => (48, 128),
                KeySpec::Hmac512 => (64, 128),
                _ => unreachable!(),
            };
            let len = min_len; // Use minimum for determinism
            let mut key = vec![0u8; len];
            aws_lc_rs::rand::fill(&mut key)?;
            Ok(CryptoKeyMaterial::Hmac { key_material: key })
        }
    }
}
```

### 9.5 Ciphertext Blob Format

For symmetric encryption (SYMMETRIC_DEFAULT), the ciphertext blob must be self-describing so that `Decrypt` can identify the key without requiring the caller to specify `KeyId`. We use the following binary format, compatible with what LocalStack uses:

```
+-----------------------------------+
| Header (68 bytes)                 |
|  +-- Key ID (36 bytes, UTF-8)    |  UUID string, e.g., "550e8400-e29b-41d4-a716-446655440000"
|  +-- IV (12 bytes)                |  AES-GCM initialization vector
|  +-- Auth Tag (16 bytes)          |  AES-GCM authentication tag
+-----------------------------------+
| Ciphertext (variable length)      |  AES-256-GCM encrypted data
+-----------------------------------+
```

Note: We use an IV length of 12 bytes (standard for AES-GCM, as recommended by NIST SP 800-38D). LocalStack uses 16 bytes (CBC mode). Since our ciphertext blobs are only consumed by our own `Decrypt`, the format is internally consistent.

```rust
/// Length of the KMS key ID (UUID format).
const KEY_ID_LEN: usize = 36;
/// AES-GCM nonce/IV length (96 bits per NIST SP 800-38D).
const IV_LEN: usize = 12;
/// AES-GCM authentication tag length (128 bits).
const TAG_LEN: usize = 16;
/// Total header length.
const HEADER_LEN: usize = KEY_ID_LEN + IV_LEN + TAG_LEN;

/// Parsed ciphertext components.
pub struct CiphertextComponents {
    /// Key ID extracted from the header.
    pub key_id: String,
    /// Initialization vector.
    pub iv: Vec<u8>,
    /// Authentication tag.
    pub tag: Vec<u8>,
    /// Encrypted data (without header).
    pub ciphertext: Vec<u8>,
}

/// Serialize ciphertext components into a ciphertext blob.
pub fn serialize_ciphertext_blob(components: &CiphertextComponents) -> Vec<u8> {
    let mut blob = Vec::with_capacity(HEADER_LEN + components.ciphertext.len());
    // Key ID is exactly 36 bytes (UUID string)
    blob.extend_from_slice(components.key_id.as_bytes());
    blob.extend_from_slice(&components.iv);
    blob.extend_from_slice(&components.tag);
    blob.extend_from_slice(&components.ciphertext);
    blob
}

/// Deserialize a ciphertext blob into its components.
pub fn deserialize_ciphertext_blob(blob: &[u8]) -> Result<CiphertextComponents, KmsServiceError> {
    if blob.len() < HEADER_LEN {
        return Err(KmsServiceError::InvalidCiphertext {
            message: "Ciphertext blob is too short".into(),
        });
    }

    let key_id = std::str::from_utf8(&blob[..KEY_ID_LEN])
        .map_err(|_| KmsServiceError::InvalidCiphertext {
            message: "Invalid key ID in ciphertext header".into(),
        })?
        .to_string();

    let iv = blob[KEY_ID_LEN..KEY_ID_LEN + IV_LEN].to_vec();
    let tag = blob[KEY_ID_LEN + IV_LEN..HEADER_LEN].to_vec();
    let ciphertext = blob[HEADER_LEN..].to_vec();

    Ok(CiphertextComponents {
        key_id,
        iv,
        tag,
        ciphertext,
    })
}
```

### 9.6 Symmetric Encryption (AES-256-GCM)

```rust
/// Encrypt plaintext with AES-256-GCM.
///
/// Returns the full ciphertext blob (header + encrypted data).
pub fn symmetric_encrypt(
    key_id: &str,
    key_material: &[u8],
    plaintext: &[u8],
    encryption_context: Option<&EncryptionContextType>,
) -> Result<Vec<u8>, KmsServiceError> {
    use aws_lc_rs::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

    // Generate random IV
    let mut iv_bytes = [0u8; IV_LEN];
    aws_lc_rs::rand::fill(&mut iv_bytes)
        .map_err(|_| KmsServiceError::internal("Failed to generate IV"))?;

    // Build AAD from encryption context
    let aad_bytes = serialize_encryption_context(encryption_context);
    let aad = Aad::from(&aad_bytes);

    // Encrypt
    let unbound_key = UnboundKey::new(&AES_256_GCM, key_material)
        .map_err(|_| KmsServiceError::internal("Invalid key material"))?;
    let key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::try_assume_unique_for_key(&iv_bytes)
        .map_err(|_| KmsServiceError::internal("Invalid nonce"))?;

    let mut in_out = plaintext.to_vec();
    let tag = key
        .seal_in_place_separate_tag(nonce, aad, &mut in_out)
        .map_err(|_| KmsServiceError::internal("Encryption failed"))?;

    let components = CiphertextComponents {
        key_id: key_id.to_string(),
        iv: iv_bytes.to_vec(),
        tag: tag.as_ref().to_vec(),
        ciphertext: in_out,
    };

    Ok(serialize_ciphertext_blob(&components))
}

/// Decrypt a ciphertext blob with AES-256-GCM.
pub fn symmetric_decrypt(
    key_material: &[u8],
    components: &CiphertextComponents,
    encryption_context: Option<&EncryptionContextType>,
) -> Result<Vec<u8>, KmsServiceError> {
    use aws_lc_rs::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

    let aad_bytes = serialize_encryption_context(encryption_context);
    let aad = Aad::from(&aad_bytes);

    let unbound_key = UnboundKey::new(&AES_256_GCM, key_material)
        .map_err(|_| KmsServiceError::internal("Invalid key material"))?;
    let key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::try_assume_unique_for_key(&components.iv)
        .map_err(|_| KmsServiceError::internal("Invalid nonce"))?;

    // Reconstruct ciphertext + tag for aws-lc-rs (it expects tag appended)
    let mut in_out = components.ciphertext.clone();
    in_out.extend_from_slice(&components.tag);

    let plaintext = key
        .open_in_place(nonce, aad, &mut in_out)
        .map_err(|_| KmsServiceError::InvalidCiphertext {
            message: "Decryption failed: invalid ciphertext or wrong key".into(),
        })?;

    Ok(plaintext.to_vec())
}
```

### 9.7 Encryption Context as AAD

Encryption context is a map of string key-value pairs used as additional authenticated data (AAD). It must be provided identically during both encryption and decryption. The serialization format matches LocalStack's implementation:

```rust
/// Serialize encryption context into bytes for use as AAD.
///
/// Keys are sorted lexicographically, then key-value pairs are
/// concatenated as raw UTF-8 bytes.
pub fn serialize_encryption_context(
    context: Option<&EncryptionContextType>,
) -> Vec<u8> {
    match context {
        Some(ctx) if !ctx.is_empty() => {
            let mut pairs: Vec<(&String, &String)> = ctx.iter().collect();
            pairs.sort_by_key(|(k, _)| *k);

            let mut aad = Vec::new();
            for (key, value) in pairs {
                // Skip the reserved key used by AWS Encryption SDK
                if key != "aws-crypto-public-key" {
                    aad.extend_from_slice(key.as_bytes());
                    aad.extend_from_slice(value.as_bytes());
                }
            }
            aad
        }
        _ => Vec::new(),
    }
}
```

### 9.8 RSA Encryption/Decryption

For asymmetric RSA keys with `ENCRYPT_DECRYPT` usage:

```rust
/// RSA OAEP encryption algorithms.
pub fn rsa_encrypt(
    public_key_der: &[u8],
    plaintext: &[u8],
    algorithm: &EncryptionAlgorithmSpec,
) -> Result<Vec<u8>, KmsServiceError> {
    // RSA OAEP with SHA-1 or SHA-256
    // For asymmetric keys, ciphertext is raw RSA ciphertext (no header)
    // because the key ID must be specified by the caller for decryption
    todo!("RSA OAEP encryption using aws-lc-rs")
}

/// RSA OAEP decryption.
pub fn rsa_decrypt(
    private_key_der: &[u8],
    ciphertext: &[u8],
    algorithm: &EncryptionAlgorithmSpec,
) -> Result<Vec<u8>, KmsServiceError> {
    todo!("RSA OAEP decryption using aws-lc-rs")
}
```

Note: For asymmetric key encryption, AWS KMS does not embed key metadata in the ciphertext. The caller must provide the `KeyId` when decrypting.

### 9.9 Digital Signatures

```rust
/// Sign a message or digest.
pub fn sign(
    key_material: &CryptoKeyMaterial,
    message: &[u8],
    message_type: &MessageType,
    signing_algorithm: &SigningAlgorithmSpec,
) -> Result<Vec<u8>, KmsServiceError> {
    match key_material {
        CryptoKeyMaterial::Rsa { private_key_der, .. } => {
            rsa_sign(private_key_der, message, message_type, signing_algorithm)
        }
        CryptoKeyMaterial::Ecc { private_key_der, curve, .. } => {
            ecdsa_sign(private_key_der, curve, message, message_type, signing_algorithm)
        }
        _ => Err(KmsServiceError::InvalidKeyUsage {
            message: "Key does not support signing".into(),
        }),
    }
}

/// Verify a signature.
pub fn verify(
    key_material: &CryptoKeyMaterial,
    message: &[u8],
    signature: &[u8],
    message_type: &MessageType,
    signing_algorithm: &SigningAlgorithmSpec,
) -> Result<bool, KmsServiceError> {
    match key_material {
        CryptoKeyMaterial::Rsa { public_key_der, .. } => {
            rsa_verify(public_key_der, message, signature, message_type, signing_algorithm)
        }
        CryptoKeyMaterial::Ecc { public_key_der, curve, .. } => {
            ecdsa_verify(public_key_der, curve, message, signature, message_type, signing_algorithm)
        }
        _ => Err(KmsServiceError::InvalidKeyUsage {
            message: "Key does not support verification".into(),
        }),
    }
}
```

### 9.10 HMAC Operations

```rust
/// Generate HMAC for a message.
pub fn generate_mac(
    key_material: &[u8],
    message: &[u8],
    mac_algorithm: &MacAlgorithmSpec,
) -> Result<Vec<u8>, KmsServiceError> {
    use aws_lc_rs::hmac;

    let algorithm = match mac_algorithm {
        MacAlgorithmSpec::HmacSha224 => hmac::HMAC_SHA224,
        MacAlgorithmSpec::HmacSha256 => hmac::HMAC_SHA256,
        MacAlgorithmSpec::HmacSha384 => hmac::HMAC_SHA384,
        MacAlgorithmSpec::HmacSha512 => hmac::HMAC_SHA512,
    };

    let key = hmac::Key::new(algorithm, key_material);
    let tag = hmac::sign(&key, message);
    Ok(tag.as_ref().to_vec())
}

/// Verify HMAC for a message.
pub fn verify_mac(
    key_material: &[u8],
    message: &[u8],
    mac: &[u8],
    mac_algorithm: &MacAlgorithmSpec,
) -> Result<bool, KmsServiceError> {
    use aws_lc_rs::hmac;

    let algorithm = match mac_algorithm {
        MacAlgorithmSpec::HmacSha224 => hmac::HMAC_SHA224,
        MacAlgorithmSpec::HmacSha256 => hmac::HMAC_SHA256,
        MacAlgorithmSpec::HmacSha384 => hmac::HMAC_SHA384,
        MacAlgorithmSpec::HmacSha512 => hmac::HMAC_SHA512,
    };

    let key = hmac::Key::new(algorithm, key_material);
    match hmac::verify(&key, message, mac) {
        Ok(()) => Ok(true),
        Err(_) => Err(KmsServiceError::KmsInvalidMac),
    }
}
```

### 9.11 Signing Algorithm Mapping

| KMS Algorithm | Key Spec | aws-lc-rs / Library |
|---------------|----------|---------------------|
| `SYMMETRIC_DEFAULT` | `SYMMETRIC_DEFAULT` | `aead::AES_256_GCM` |
| `RSAES_OAEP_SHA_1` | `RSA_*` | RSA OAEP with SHA-1 |
| `RSAES_OAEP_SHA_256` | `RSA_*` | RSA OAEP with SHA-256 |
| `RSASSA_PSS_SHA_256` | `RSA_*` | `signature::RSA_PSS_SHA256` |
| `RSASSA_PSS_SHA_384` | `RSA_*` | `signature::RSA_PSS_SHA384` |
| `RSASSA_PSS_SHA_512` | `RSA_*` | `signature::RSA_PSS_SHA512` |
| `RSASSA_PKCS1_V1_5_SHA_256` | `RSA_*` | `signature::RSA_PKCS1_SHA256` |
| `RSASSA_PKCS1_V1_5_SHA_384` | `RSA_*` | `signature::RSA_PKCS1_SHA384` |
| `RSASSA_PKCS1_V1_5_SHA_512` | `RSA_*` | `signature::RSA_PKCS1_SHA512` |
| `ECDSA_SHA_256` | `ECC_NIST_P256`, `ECC_SECG_P256K1` | `signature::ECDSA_P256_SHA256_ASN1_SIGNING` / `k256` |
| `ECDSA_SHA_384` | `ECC_NIST_P384` | `signature::ECDSA_P384_SHA384_ASN1_SIGNING` |
| `ECDSA_SHA_512` | `ECC_NIST_P521` | Requires P-521 support |
| `HMAC_SHA_224` | `HMAC_224` | `hmac::HMAC_SHA224` |
| `HMAC_SHA_256` | `HMAC_256` | `hmac::HMAC_SHA256` |
| `HMAC_SHA_384` | `HMAC_384` | `hmac::HMAC_SHA384` |
| `HMAC_SHA_512` | `HMAC_512` | `hmac::HMAC_SHA512` |

### 9.12 Key Spec / Usage / Algorithm Compatibility Matrix

| Key Spec | Key Usage | Allowed Algorithms |
|----------|-----------|-------------------|
| `SYMMETRIC_DEFAULT` | `ENCRYPT_DECRYPT` | `SYMMETRIC_DEFAULT` |
| `RSA_2048` | `ENCRYPT_DECRYPT` | `RSAES_OAEP_SHA_1`, `RSAES_OAEP_SHA_256` |
| `RSA_2048` | `SIGN_VERIFY` | `RSASSA_PSS_SHA_*`, `RSASSA_PKCS1_V1_5_SHA_*` |
| `RSA_3072` | `ENCRYPT_DECRYPT` | `RSAES_OAEP_SHA_1`, `RSAES_OAEP_SHA_256` |
| `RSA_3072` | `SIGN_VERIFY` | `RSASSA_PSS_SHA_*`, `RSASSA_PKCS1_V1_5_SHA_*` |
| `RSA_4096` | `ENCRYPT_DECRYPT` | `RSAES_OAEP_SHA_1`, `RSAES_OAEP_SHA_256` |
| `RSA_4096` | `SIGN_VERIFY` | `RSASSA_PSS_SHA_*`, `RSASSA_PKCS1_V1_5_SHA_*` |
| `ECC_NIST_P256` | `SIGN_VERIFY` | `ECDSA_SHA_256` |
| `ECC_NIST_P384` | `SIGN_VERIFY` | `ECDSA_SHA_384` |
| `ECC_NIST_P521` | `SIGN_VERIFY` | `ECDSA_SHA_512` |
| `ECC_SECG_P256K1` | `SIGN_VERIFY` | `ECDSA_SHA_256` |
| `HMAC_224` | `GENERATE_VERIFY_MAC` | `HMAC_SHA_224` |
| `HMAC_256` | `GENERATE_VERIFY_MAC` | `HMAC_SHA_256` |
| `HMAC_384` | `GENERATE_VERIFY_MAC` | `HMAC_SHA_384` |
| `HMAC_512` | `GENERATE_VERIFY_MAC` | `HMAC_SHA_512` |

The provider must validate that the requested algorithm is compatible with the key's spec and usage before performing any cryptographic operation.

---

## 10. Storage Engine Design

### 10.1 Overview

The storage model consists of three primary `DashMap`-based stores per account/region: keys, aliases, and grants. Key policies are stored inline with the key.

### 10.2 Core Data Structures

```rust
/// Top-level KMS store, scoped per account+region via rustack-core.
pub struct KmsStore {
    /// All KMS keys, keyed by Key ID (UUID).
    keys: DashMap<String, KmsKey>,

    /// Alias name -> Key ID mapping.
    /// Key: alias name (e.g., "alias/my-key"), Value: Key ID (UUID).
    aliases: DashMap<String, String>,

    /// All grants, keyed by Grant ID.
    grants: DashMap<String, KmsGrant>,
}

/// A KMS key with metadata, crypto material, and state.
pub struct KmsKey {
    /// Key metadata (KeyId, Arn, KeySpec, KeyUsage, etc.).
    pub metadata: KeyMetadata,

    /// Cryptographic key material.
    pub crypto_key: CryptoKeyMaterial,

    /// Key policy (JSON string).
    pub policy: String,

    /// Whether automatic key rotation is enabled.
    pub rotation_enabled: bool,

    /// Tags on the key.
    pub tags: Vec<Tag>,

    /// Key state (Enabled, Disabled, PendingDeletion, PendingImport).
    pub state: KeyState,

    /// Scheduled deletion date (if state is PendingDeletion).
    pub deletion_date: Option<f64>,
}

/// KeyMetadata as it appears in API responses.
/// This is auto-generated by Smithy codegen, but here is the logical structure:
pub struct KeyMetadata {
    pub key_id: String,
    pub arn: String,
    pub creation_date: f64,
    pub enabled: bool,
    pub description: String,
    pub key_usage: KeyUsageType,
    pub key_state: KeyState,
    pub key_spec: KeySpec,
    pub encryption_algorithms: Vec<EncryptionAlgorithmSpec>,
    pub signing_algorithms: Vec<SigningAlgorithmSpec>,
    pub mac_algorithms: Vec<MacAlgorithmSpec>,
    pub key_manager: KeyManagerType,
    pub origin: OriginType,
    pub multi_region: bool,
    // ... additional fields
}

/// A KMS grant.
pub struct KmsGrant {
    pub grant_id: String,
    pub grant_token: String,
    pub key_id: String,
    pub grantee_principal: String,
    pub retiring_principal: Option<String>,
    pub operations: Vec<GrantOperation>,
    pub constraints: Option<GrantConstraints>,
    pub name: Option<String>,
    pub creation_date: f64,
    pub issuing_account: String,
}
```

### 10.3 Key State Machine

Keys follow a strict state machine:

```
    CreateKey
        |
        v
  +-----------+
  |  Enabled  |<---------+
  +-----------+           |
     |      |             |
     |  DisableKey    EnableKey
     |      |             |
     |      v             |
     |  +----------+      |
     |  | Disabled |------+
     |  +----------+
     |      |
     |  ScheduleKeyDeletion
     |      |
     |      v
     |  +------------------+
     +->| PendingDeletion  |
        +------------------+
             |
        CancelKeyDeletion
             |
             v
        +----------+
        | Disabled |
        +----------+
```

State transition rules:

| From | To | Trigger |
|------|----|---------|
| Enabled | Disabled | `DisableKey` |
| Disabled | Enabled | `EnableKey` |
| Enabled | PendingDeletion | `ScheduleKeyDeletion` |
| Disabled | PendingDeletion | `ScheduleKeyDeletion` |
| PendingDeletion | Disabled | `CancelKeyDeletion` |

Operations that require key state Enabled:
- `Encrypt`, `Decrypt`, `ReEncrypt`
- `GenerateDataKey`, `GenerateDataKeyWithoutPlaintext`
- `GenerateDataKeyPair`, `GenerateDataKeyPairWithoutPlaintext`
- `Sign`, `Verify`
- `GenerateMac`, `VerifyMac`
- `GetPublicKey`

Operations that work on Disabled keys:
- `DescribeKey`, `ListKeys`
- `EnableKey`
- `ScheduleKeyDeletion`
- `TagResource`, `UntagResource`, `ListResourceTags`
- `GetKeyPolicy`, `PutKeyPolicy`, `ListKeyPolicies`
- `CreateGrant`, `ListGrants`, `RetireGrant`, `RevokeGrant`
- `EnableKeyRotation`, `DisableKeyRotation`, `GetKeyRotationStatus`

Operations that fail on PendingDeletion keys (with `KMSInvalidStateException`):
- All cryptographic operations
- `EnableKey`, `DisableKey`
- `ScheduleKeyDeletion` (already pending)
- `CreateGrant`

### 10.4 Key ID Resolution

Keys can be referenced by four formats. The provider must resolve all of them:

```rust
/// Resolve a key identifier to a Key ID (UUID).
///
/// Supports:
///   - Key ID (UUID): "550e8400-e29b-41d4-a716-446655440000"
///   - Key ARN: "arn:aws:kms:us-east-1:000000000000:key/550e8400-..."
///   - Alias name: "alias/my-key"
///   - Alias ARN: "arn:aws:kms:us-east-1:000000000000:alias/my-key"
pub fn resolve_key_id(
    &self,
    key_identifier: &str,
    account_id: &str,
    region: &str,
) -> Result<String, KmsServiceError> {
    // 1. Try as alias name ("alias/...")
    if key_identifier.starts_with("alias/") {
        return self.resolve_alias(key_identifier);
    }

    // 2. Try as alias ARN ("arn:aws:kms:...:alias/...")
    if key_identifier.contains(":alias/") {
        let alias_name = extract_alias_from_arn(key_identifier)?;
        return self.resolve_alias(&alias_name);
    }

    // 3. Try as key ARN ("arn:aws:kms:...:key/...")
    if key_identifier.starts_with("arn:") {
        let (arn_account, arn_region, key_id) = parse_key_arn(key_identifier)?;
        // Validate region matches
        if arn_region != region {
            return Err(KmsServiceError::NotFoundException {
                message: format!("Key '{key_identifier}' does not exist"),
            });
        }
        return Ok(key_id);
    }

    // 4. Treat as Key ID (UUID)
    Ok(key_identifier.to_string())
}
```

### 10.5 Alias Management

```rust
/// Reserved AWS-managed aliases that are auto-created on first reference.
const RESERVED_ALIASES: &[&str] = &[
    "alias/aws/acm",
    "alias/aws/dynamodb",
    "alias/aws/ebs",
    "alias/aws/elasticfilesystem",
    "alias/aws/es",
    "alias/aws/glue",
    "alias/aws/kinesisvideo",
    "alias/aws/lambda",
    "alias/aws/rds",
    "alias/aws/redshift",
    "alias/aws/s3",
    "alias/aws/secretsmanager",
    "alias/aws/ssm",
    "alias/aws/xray",
];
```

When an `alias/aws/*` alias is referenced but does not exist, the provider auto-creates both the alias and an associated symmetric key. This matches AWS behavior where AWS-managed keys are created lazily.

### 10.6 Concurrency Model

KMS has no real-time constraints, no streaming, and no background processing. A `DashMap` provides sufficient concurrent access:

- **Reads** (DescribeKey, ListKeys, ListAliases, ListGrants, GetKeyPolicy, GetKeyRotationStatus, GetPublicKey): lock-free concurrent reads
- **Writes** (CreateKey, EnableKey, DisableKey, Encrypt/Decrypt, CreateAlias, TagResource, CreateGrant): per-entry write locks via DashMap
- **Crypto operations** (Encrypt, Decrypt, Sign, Verify, GenerateMac): read-lock the key, perform crypto outside the lock

No actors, no channels, no background tasks. Key deletion scheduling is tracked by a timestamp; the key transitions to PendingDeletion immediately and would be purged by a background task in production, but for local dev we simply prevent crypto operations on PendingDeletion keys.

---

## 11. Core Business Logic

### 11.1 Provider

```rust
/// Main KMS provider implementing all ~35 operations.
pub struct RustackKms {
    pub(crate) store: Arc<KmsStore>,
    pub(crate) config: Arc<KmsConfig>,
}

impl RustackKms {
    pub fn new(config: KmsConfig) -> Self {
        Self {
            store: Arc::new(KmsStore::new()),
            config: Arc::new(config),
        }
    }
}
```

### 11.2 Operations

#### Phase 0: Key Management + Symmetric Encryption + Aliases (16 operations)

**CreateKey** -- Create a new KMS key.

1. Validate `KeySpec` (default: `SYMMETRIC_DEFAULT`)
2. Validate `KeyUsage` (default: `ENCRYPT_DECRYPT`); enforce valid spec/usage combinations
3. Validate `KeySpec`/`KeyUsage` compatibility (e.g., HMAC requires `GENERATE_VERIFY_MAC`)
4. Generate key material using `generate_key_material(key_spec)`
5. Generate UUID key ID
6. Build `KeyMetadata` with ARN, creation timestamp, algorithm lists
7. Validate and store tags (max 50, no `aws:` prefix, no duplicate keys)
8. Store key with default policy
9. Store `Origin: AWS_KMS` (EXTERNAL not supported in MVP)
10. Return `KeyMetadata`

**DescribeKey** -- Retrieve key metadata.

1. Resolve key identifier (UUID, ARN, alias name, alias ARN)
2. Look up key; return `NotFoundException` if missing
3. Return `KeyMetadata` (no key material)

**ListKeys** -- List all keys.

1. Iterate keys in store for the account/region
2. Return `KeyListEntry` objects (KeyId, KeyArn only)
3. Support pagination with `Limit` and `Marker`

**EnableKey** -- Re-enable a disabled key.

1. Resolve and look up key
2. Validate state is `Disabled` (not `PendingDeletion`)
3. Set state to `Enabled`, set `Enabled: true`

**DisableKey** -- Disable a key.

1. Resolve and look up key
2. Validate state is `Enabled`
3. Set state to `Disabled`, set `Enabled: false`

**ScheduleKeyDeletion** -- Schedule key for deletion.

1. Resolve and look up key
2. Validate state is not `PendingDeletion`
3. Set state to `PendingDeletion`, set `Enabled: false`
4. Calculate `DeletionDate` (default: 30 days; range 7-30; `PendingWindowInDays` parameter)
5. Return `{ KeyId, DeletionDate, KeyState, PendingWindowInDays }`

**CancelKeyDeletion** -- Cancel scheduled deletion.

1. Resolve and look up key
2. Validate state is `PendingDeletion`
3. Set state to `Disabled` (not Enabled), clear `DeletionDate`
4. Return `{ KeyId }`

**UpdateKeyDescription** -- Update key description.

1. Resolve and look up key
2. Update `Description` field in metadata

**Encrypt** -- Encrypt plaintext.

1. Resolve and look up key
2. Validate key state is `Enabled`; return `DisabledException` if disabled
3. Validate key usage is `ENCRYPT_DECRYPT`; return `InvalidKeyUsageException` if wrong
4. Validate plaintext size (max 4096 bytes for symmetric, varies for RSA)
5. For `SYMMETRIC_DEFAULT`: call `symmetric_encrypt`, return ciphertext blob
6. For `RSAES_OAEP_*`: call `rsa_encrypt`, return raw RSA ciphertext
7. Return `{ CiphertextBlob, KeyId, EncryptionAlgorithm }`

**Decrypt** -- Decrypt ciphertext.

1. For symmetric: parse ciphertext blob header to extract key ID
2. If caller provided `KeyId`, validate it matches the key in the ciphertext header
3. Resolve and look up key
4. Validate key state is `Enabled`
5. Validate key usage is `ENCRYPT_DECRYPT`
6. For `SYMMETRIC_DEFAULT`: call `symmetric_decrypt` with encryption context
7. For `RSAES_OAEP_*`: call `rsa_decrypt` (requires caller to specify `KeyId`)
8. Return `{ Plaintext, KeyId, EncryptionAlgorithm }`

**ReEncrypt** -- Decrypt with source key, re-encrypt with destination key.

1. Decrypt using source key (same as Decrypt)
2. Encrypt using destination key (same as Encrypt)
3. Return `{ CiphertextBlob, SourceKeyId, KeyId, SourceEncryptionAlgorithm, DestinationEncryptionAlgorithm }`

**GenerateDataKey** -- Generate a data key for envelope encryption.

1. Resolve and look up key
2. Validate key state is `Enabled`, usage is `ENCRYPT_DECRYPT`
3. Generate random data key (32 bytes for `AES_256`, 16 bytes for `AES_128`, or `NumberOfBytes`)
4. Encrypt the data key using the KMS key (produces ciphertext blob)
5. Return `{ Plaintext, CiphertextBlob, KeyId }` (both plaintext and encrypted)

**GenerateDataKeyWithoutPlaintext** -- Same as `GenerateDataKey` but without returning plaintext.

1. Same as `GenerateDataKey` steps 1-4
2. Return `{ CiphertextBlob, KeyId }` (encrypted only)

**CreateAlias** -- Create an alias for a key.

1. Validate alias name starts with `alias/`
2. Validate alias does not start with `alias/aws/` (reserved)
3. Validate alias does not already exist; return `AlreadyExistsException` if so
4. Resolve target key ID
5. Store alias -> key ID mapping

**DeleteAlias** -- Delete an alias.

1. Validate alias exists; return `NotFoundException` if not
2. Remove alias from store

**ListAliases** -- List all aliases (optionally filtered by key ID).

1. Iterate aliases in store
2. If `KeyId` is provided, filter to aliases pointing to that key
3. Build `AliasListEntry` objects with `AliasName`, `AliasArn`, `TargetKeyId`, `CreationDate`
4. Support pagination with `Limit` and `Marker`

**UpdateAlias** -- Point an alias to a different key.

1. Validate alias exists
2. Validate alias does not start with `alias/aws/`
3. Resolve new target key ID
4. Update alias -> key ID mapping

#### Phase 1: Asymmetric, Signing, HMAC, Tags, Policies (14 operations)

**Sign** -- Create a digital signature.

1. Resolve and look up key
2. Validate key state is `Enabled`, usage is `SIGN_VERIFY`
3. Validate signing algorithm is compatible with key spec
4. If `MessageType` is `DIGEST`, validate digest length matches algorithm's hash output
5. If `MessageType` is `RAW`, hash the message internally before signing
6. Call `sign()` with key material
7. Return `{ Signature, KeyId, SigningAlgorithm }`

**Verify** -- Verify a digital signature.

1. Resolve and look up key
2. Validate key state is `Enabled`, usage is `SIGN_VERIFY`
3. Validate signing algorithm is compatible with key spec
4. Call `verify()` with key material
5. If signature is invalid, raise `KMSInvalidSignatureException`
6. Return `{ SignatureValid, KeyId, SigningAlgorithm }`

**GetPublicKey** -- Get the public key of an asymmetric key.

1. Resolve and look up key
2. Validate key is asymmetric (RSA or ECC)
3. Return `{ PublicKey (DER), KeyId, KeySpec, KeyUsage, EncryptionAlgorithms, SigningAlgorithms }`

**GenerateMac** -- Generate HMAC.

1. Resolve and look up key
2. Validate key state is `Enabled`, usage is `GENERATE_VERIFY_MAC`
3. Validate MAC algorithm is compatible with key spec
4. Call `generate_mac()` with key material
5. Return `{ Mac, KeyId, MacAlgorithm }`

**VerifyMac** -- Verify HMAC.

1. Resolve and look up key
2. Validate key state is `Enabled`, usage is `GENERATE_VERIFY_MAC`
3. Validate MAC algorithm is compatible with key spec
4. Call `verify_mac()` with key material
5. If invalid, raise `KMSInvalidMacException`
6. Return `{ MacValid, KeyId, MacAlgorithm }`

**GenerateDataKeyPair** -- Generate asymmetric data key pair.

1. Resolve and look up KMS key (must be symmetric, `ENCRYPT_DECRYPT`)
2. Generate new RSA or ECC key pair (based on `KeyPairSpec`)
3. Encrypt private key DER bytes using the KMS key
4. Return `{ PrivateKeyPlaintext, PrivateKeyCiphertextBlob, PublicKey, KeyId, KeyPairSpec }`

**GenerateDataKeyPairWithoutPlaintext** -- Same without plaintext private key.

1. Same as `GenerateDataKeyPair` steps 1-3
2. Return `{ PrivateKeyCiphertextBlob, PublicKey, KeyId, KeyPairSpec }`

**TagResource** -- Add tags to a key.

1. Resolve and look up key
2. Validate tags (max 50 total, no `aws:` prefix, key max 128 chars, value max 256 chars, no duplicates)
3. Merge new tags (overwrite existing keys)

**UntagResource** -- Remove tags from a key.

1. Resolve and look up key
2. Remove specified tag keys

**ListResourceTags** -- List tags on a key.

1. Resolve and look up key
2. Return `{ Tags }`

**GetKeyPolicy** -- Get key policy.

1. Resolve and look up key
2. Return `{ Policy }` (JSON string)

**PutKeyPolicy** -- Set key policy.

1. Resolve and look up key
2. Store policy string (no validation of policy content)

**ListKeyPolicies** -- List policy names.

1. Return `{ PolicyNames: ["default"], Truncated: false }`
2. AWS currently only supports the "default" policy name

**GenerateRandom** -- Generate random bytes.

1. Validate `NumberOfBytes` (1-1024)
2. Generate random bytes using `aws_lc_rs::rand::fill`
3. Return `{ Plaintext }`

#### Phase 2: Grants + Key Rotation (6 operations)

**CreateGrant** -- Create a grant on a key.

1. Resolve and look up key
2. Generate grant ID and grant token
3. Validate `GranteePrincipal` is provided
4. Store grant with operations, constraints, name
5. If `Name` is provided, enforce uniqueness per key
6. Return `{ GrantId, GrantToken }`

**ListGrants** -- List grants on a key.

1. Resolve and look up key
2. Filter grants by key ID
3. Support pagination with `Limit` and `Marker`
4. Return `{ Grants }`

**RetireGrant** -- Retire a grant.

1. Look up grant by `GrantToken` or by `GrantId` + `KeyId`
2. Remove grant from store
3. Return `{}`

**RevokeGrant** -- Revoke a grant.

1. Resolve key and look up grant by `GrantId`
2. Validate grant belongs to the specified key
3. Remove grant from store
4. Return `{}`

**ListRetirableGrants** -- List grants retirable by a principal.

1. Filter all grants by `RetiringPrincipal`
2. Return `{ Grants }`

**EnableKeyRotation** / **DisableKeyRotation** / **GetKeyRotationStatus**

1. Resolve and look up key
2. Validate key is symmetric with `ENCRYPT_DECRYPT` usage
3. Set/get `rotation_enabled` flag
4. `GetKeyRotationStatus` returns `{ KeyRotationEnabled, RotationPeriodInDays }`

### 11.3 ARN Construction

```rust
fn key_arn(region: &str, account_id: &str, key_id: &str) -> String {
    format!("arn:aws:kms:{region}:{account_id}:key/{key_id}")
}

fn alias_arn(region: &str, account_id: &str, alias_name: &str) -> String {
    format!("arn:aws:kms:{region}:{account_id}:{alias_name}")
}
```

### 11.4 Validation Rules

| Field | Rule |
|-------|------|
| Key ID | UUID format: `[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}` |
| Multi-region key ID | `mrk-[a-f0-9]{32}` (not supported in MVP) |
| Alias name | Must start with `alias/`, 1-256 chars, `[a-zA-Z0-9:/_-]` |
| Reserved aliases | `alias/aws/*` cannot be created/deleted/updated by users |
| Tag key | 1-128 chars, no `aws:` prefix (case-insensitive) |
| Tag value | 0-256 chars |
| Tags per key | Max 50 |
| Plaintext size (symmetric) | Max 4096 bytes |
| Plaintext size (RSA) | Depends on key size and algorithm (e.g., RSA-2048 OAEP-SHA256: max 190 bytes) |
| Description | 0-8192 chars |
| PendingWindowInDays | 7-30 (default: 30) |
| NumberOfBytes (GenerateRandom) | 1-1024 |
| Grant name | 1-256 chars, `[a-zA-Z0-9:/_-]` |
| Policy | Valid JSON string (stored, not parsed for authorization) |

---

## 12. Error Handling

### 12.1 Error Types

```rust
/// KMS service errors mapped to API error types.
pub enum KmsServiceError {
    /// Key does not exist.
    NotFoundException { message: String },

    /// Key is disabled.
    DisabledException { message: String },

    /// Key is in an invalid state for the requested operation.
    KmsInvalidStateException { message: String },

    /// Key usage does not match the operation.
    InvalidKeyUsageException { message: String },

    /// Ciphertext is invalid or corrupted.
    InvalidCiphertext { message: String },

    /// Signature verification failed.
    KmsInvalidSignatureException,

    /// MAC verification failed.
    KmsInvalidMac,

    /// Alias already exists.
    AlreadyExistsException { message: String },

    /// Validation error (parameter format, range, etc.).
    ValidationException { message: String },

    /// Tag-related errors.
    TagException { message: String },

    /// Grant-related errors.
    InvalidGrantIdException { message: String },

    /// Dependency timeout (not used in local dev).
    DependencyTimeoutException { message: String },

    /// Operation not supported.
    UnsupportedOperationException { message: String },

    /// Limit exceeded (e.g., max tags).
    LimitExceededException { message: String },

    /// Internal error.
    KmsInternalException { message: String },
}
```

### 12.2 Error Mapping

```rust
impl KmsServiceError {
    /// Map to HTTP status code and __type string.
    pub fn to_error_response(&self) -> (u16, &'static str, String) {
        match self {
            Self::NotFoundException { message } =>
                (400, "NotFoundException", message.clone()),
            Self::DisabledException { message } =>
                (400, "DisabledException", message.clone()),
            Self::KmsInvalidStateException { message } =>
                (400, "KMSInvalidStateException", message.clone()),
            Self::InvalidKeyUsageException { message } =>
                (400, "InvalidKeyUsageException", message.clone()),
            Self::InvalidCiphertext { message } =>
                (400, "InvalidCiphertextException", message.clone()),
            Self::KmsInvalidSignatureException =>
                (400, "KMSInvalidSignatureException", String::new()),
            Self::KmsInvalidMac =>
                (400, "KMSInvalidMacException", String::new()),
            Self::AlreadyExistsException { message } =>
                (400, "AlreadyExistsException", message.clone()),
            Self::ValidationException { message } =>
                (400, "ValidationException", message.clone()),
            Self::TagException { message } =>
                (400, "TagException", message.clone()),
            Self::InvalidGrantIdException { message } =>
                (400, "InvalidGrantIdException", message.clone()),
            Self::UnsupportedOperationException { message } =>
                (400, "UnsupportedOperationException", message.clone()),
            Self::LimitExceededException { message } =>
                (400, "LimitExceededException", message.clone()),
            Self::KmsInternalException { message } =>
                (500, "KMSInternalException", message.clone()),
            Self::DependencyTimeoutException { message } =>
                (500, "DependencyTimeoutException", message.clone()),
        }
    }
}
```

### 12.3 Error Response Format

```json
{
    "__type": "NotFoundException",
    "message": "Key 'arn:aws:kms:us-east-1:000000000000:key/fake-id' does not exist"
}
```

KMS uses short error type names (no namespace prefix), same as SSM.

---

## 13. Server Integration

### 13.1 Feature Gate

KMS support is gated behind a cargo feature:

```toml
# apps/rustack/Cargo.toml
[features]
default = ["s3", "dynamodb", "sqs", "ssm", "kms"]
s3 = ["dep:rustack-s3-core", "dep:rustack-s3-http"]
dynamodb = ["dep:rustack-dynamodb-core", "dep:rustack-dynamodb-http"]
sqs = ["dep:rustack-sqs-core", "dep:rustack-sqs-http"]
ssm = ["dep:rustack-ssm-core", "dep:rustack-ssm-http"]
kms = ["dep:rustack-kms-core", "dep:rustack-kms-http"]
```

### 13.2 Gateway Registration

KMS is registered in the gateway before S3 (S3 is the catch-all):

```rust
// In gateway setup
let mut services: Vec<Box<dyn ServiceRouter>> = Vec::new();

#[cfg(feature = "dynamodb")]
services.push(Box::new(DynamoDBServiceRouter::new(dynamodb_service)));

#[cfg(feature = "sqs")]
services.push(Box::new(SqsServiceRouter::new(sqs_service)));

#[cfg(feature = "ssm")]
services.push(Box::new(SsmServiceRouter::new(ssm_service)));

#[cfg(feature = "kms")]
services.push(Box::new(KmsServiceRouter::new(kms_service)));

// S3 is always last (catch-all for requests without X-Amz-Target)
#[cfg(feature = "s3")]
services.push(Box::new(S3ServiceRouter::new(s3_service)));
```

### 13.3 Health Endpoint

```json
GET /_localstack/health

{
    "services": {
        "s3": "available",
        "dynamodb": "available",
        "sqs": "available",
        "ssm": "available",
        "kms": "available"
    },
    "version": "0.3.0"
}
```

### 13.4 Configuration

```rust
pub struct KmsConfig {
    /// Skip SigV4 signature validation (default: true for local dev).
    pub skip_signature_validation: bool,
    /// Default region.
    pub default_region: String,
    /// Default account ID.
    pub default_account_id: String,
}

impl KmsConfig {
    pub fn from_env() -> Self {
        Self {
            skip_signature_validation: env_bool("KMS_SKIP_SIGNATURE_VALIDATION", true),
            default_region: env_str("DEFAULT_REGION", "us-east-1"),
            default_account_id: env_str("DEFAULT_ACCOUNT_ID", "000000000000"),
        }
    }
}
```

### 13.5 Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `GATEWAY_LISTEN` | `0.0.0.0:4566` | Bind address (shared) |
| `KMS_SKIP_SIGNATURE_VALIDATION` | `true` | Skip SigV4 for KMS |
| `DEFAULT_REGION` | `us-east-1` | Default AWS region |
| `DEFAULT_ACCOUNT_ID` | `000000000000` | Default account for ARNs |

---

## 14. Testing Strategy

### 14.1 Unit Tests

Each module tested in isolation:

- **Ciphertext blob**: Test serialize/deserialize round-trip, invalid blob handling, header parsing
- **Crypto engine**: Test AES-256-GCM encrypt/decrypt round-trip, RSA keygen + encrypt/decrypt, ECDSA sign/verify, HMAC generate/verify
- **Encryption context**: Test serialization, sorted key ordering, empty context
- **Key state machine**: Test all valid transitions, reject invalid transitions
- **Key ID resolution**: Test UUID, ARN, alias name, alias ARN, invalid formats
- **Validation**: Key spec/usage compatibility, tag limits, alias name format, plaintext size limits
- **Alias management**: Create, delete, update, list, reserved alias auto-creation

### 14.2 Integration Tests with aws-sdk-kms

```rust
// tests/integration/kms_tests.rs
#[tokio::test]
#[ignore]
async fn test_kms_create_describe_list_key() {
    let client = aws_sdk_kms::Client::new(&config);

    // CreateKey
    let key = client.create_key()
        .description("test key")
        .key_usage(KeyUsageType::EncryptDecrypt)
        .send().await.unwrap();
    let key_id = key.key_metadata().unwrap().key_id();

    // DescribeKey
    let desc = client.describe_key()
        .key_id(key_id)
        .send().await.unwrap();
    assert_eq!(desc.key_metadata().unwrap().key_id(), key_id);

    // ListKeys
    let list = client.list_keys().send().await.unwrap();
    assert!(list.keys().iter().any(|k| k.key_id() == Some(key_id)));
}

#[tokio::test]
#[ignore]
async fn test_kms_encrypt_decrypt_symmetric() {
    let client = aws_sdk_kms::Client::new(&config);
    let key_id = create_test_key(&client).await;

    let plaintext = b"hello world";
    let encrypt = client.encrypt()
        .key_id(&key_id)
        .plaintext(Blob::new(plaintext.as_slice()))
        .send().await.unwrap();

    let decrypt = client.decrypt()
        .ciphertext_blob(encrypt.ciphertext_blob().unwrap().clone())
        .send().await.unwrap();

    assert_eq!(decrypt.plaintext().unwrap().as_ref(), plaintext);
}

#[tokio::test]
#[ignore]
async fn test_kms_generate_data_key() {
    let client = aws_sdk_kms::Client::new(&config);
    let key_id = create_test_key(&client).await;

    let data_key = client.generate_data_key()
        .key_id(&key_id)
        .key_spec(DataKeySpec::Aes256)
        .send().await.unwrap();

    // Plaintext data key is 32 bytes for AES-256
    assert_eq!(data_key.plaintext().unwrap().as_ref().len(), 32);

    // Can decrypt the ciphertext blob to get the same key
    let decrypted = client.decrypt()
        .ciphertext_blob(data_key.ciphertext_blob().unwrap().clone())
        .send().await.unwrap();

    assert_eq!(
        decrypted.plaintext().unwrap().as_ref(),
        data_key.plaintext().unwrap().as_ref()
    );
}

#[tokio::test]
#[ignore]
async fn test_kms_alias_lifecycle() {
    let client = aws_sdk_kms::Client::new(&config);
    let key_id = create_test_key(&client).await;

    // Create alias
    client.create_alias()
        .alias_name("alias/test-key")
        .target_key_id(&key_id)
        .send().await.unwrap();

    // Encrypt using alias
    let encrypted = client.encrypt()
        .key_id("alias/test-key")
        .plaintext(Blob::new(b"test"))
        .send().await.unwrap();

    // Delete alias
    client.delete_alias()
        .alias_name("alias/test-key")
        .send().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn test_kms_key_state_machine() {
    let client = aws_sdk_kms::Client::new(&config);
    let key_id = create_test_key(&client).await;

    // Disable key
    client.disable_key().key_id(&key_id).send().await.unwrap();
    let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
    assert!(!desc.key_metadata().unwrap().enabled());

    // Encrypt should fail on disabled key
    let err = client.encrypt()
        .key_id(&key_id)
        .plaintext(Blob::new(b"test"))
        .send().await;
    assert!(err.is_err());

    // Enable key
    client.enable_key().key_id(&key_id).send().await.unwrap();

    // Schedule deletion
    client.schedule_key_deletion()
        .key_id(&key_id)
        .pending_window_in_days(7)
        .send().await.unwrap();

    // Cancel deletion
    client.cancel_key_deletion().key_id(&key_id).send().await.unwrap();
    let desc = client.describe_key().key_id(&key_id).send().await.unwrap();
    assert_eq!(desc.key_metadata().unwrap().key_state(), &KeyState::Disabled);
}
```

### 14.3 Third-Party Test Suites

#### 14.3.1 LocalStack Test Suite (Primary)

The most comprehensive open-source KMS test suite. Already vendored at `vendors/localstack/tests/aws/services/kms/`.

- **`test_kms.py`** -- 3,031 lines covering:
  - Key lifecycle (create, describe, list, enable/disable, schedule/cancel deletion)
  - Symmetric encryption/decryption with encryption context
  - RSA encryption/decryption (OAEP SHA-1/SHA-256)
  - Digital signatures (RSA PSS/PKCS1, ECDSA P-256/P-384/secp256k1)
  - HMAC generation and verification (SHA-224/256/384/512)
  - GenerateDataKey / GenerateDataKeyWithoutPlaintext
  - GenerateDataKeyPair / GenerateDataKeyPairWithoutPlaintext
  - ReEncrypt (symmetric + RSA)
  - Alias management (create, delete, update, list, use in encryption)
  - Grant management (create, list, revoke, retire)
  - Key policies (get, put, list)
  - Tag management (create with tags, tag/untag, list, validation)
  - Key rotation status
  - GenerateRandom
  - Multi-region keys (not in MVP scope)
  - Key import (not in MVP scope)
  - Error scenarios (invalid key ID, wrong key usage, disabled keys, etc.)
- **Framework**: pytest with snapshot testing

Adaptation strategy: run the Python test suite against Rustack's KMS endpoint, track pass/fail counts, progressively fix failures. Tests marked `@markers.aws.only_localstack` can be skipped (they test LocalStack-specific features like custom key material via tags).

```makefile
test-kms-localstack:
	@cd vendors/localstack && python -m pytest tests/aws/services/kms/test_kms.py \
		--endpoint-url=http://localhost:4566 -v
```

| Test Area | Approximate Count | Phase |
|-----------|-------------------|-------|
| Key lifecycle (create, describe, list, enable/disable/delete) | ~15 | Phase 0 |
| Symmetric encrypt/decrypt | ~8 | Phase 0 |
| GenerateDataKey / GenerateDataKeyWithoutPlaintext | ~6 | Phase 0 |
| Alias management | ~8 | Phase 0 |
| Tag management | ~12 | Phase 0 |
| Signing (RSA + ECDSA) | ~10 | Phase 1 |
| HMAC (generate/verify) | ~10 | Phase 1 |
| RSA asymmetric encrypt/decrypt | ~6 | Phase 1 |
| Key policies | ~3 | Phase 1 |
| GenerateDataKeyPair | ~6 | Phase 1 |
| Grants | ~8 | Phase 2 |
| Key rotation | ~10 | Phase 2 |
| GenerateRandom | ~4 | Phase 0 |
| Error scenarios | ~15 | All phases |

#### 14.3.2 Moto KMS Test Suite (Secondary Validation)

The moto project ([`getmoto/moto`](https://github.com/getmoto/moto)) contains comprehensive KMS tests:

- **Repository**: https://github.com/getmoto/moto
- **Location**: `tests/test_kms/test_kms_boto3.py`, `tests/test_kms/test_utils.py`
- **Coverage**: Key creation, encryption/decryption, key policies, grants, aliases, tags, key rotation, GenerateDataKey, Sign/Verify
- **Framework**: pytest with `@mock_kms` decorator

Moto tests are designed for in-process mocking and require adaptation to run against an HTTP endpoint. They serve as a secondary reference for expected behavior and error message formats.

#### 14.3.3 local-kms Test Suite (Tertiary Validation)

Neil Smith's `local-kms` project provides a Go-based KMS mock with functional tests:

- **Repository**: https://github.com/nsmithuk/local-kms
- **Tests**: `tests/functional/` directory
- **Coverage**: Key operations, encryption/decryption, signing, aliases, seeding
- **Key feature**: Tests can run against both local-kms and real AWS KMS, enabling cross-validation
- **Framework**: Python-based functional tests with custom KMS client

The local-kms test client (`tests/functional/KmsClient/KmsClient.py`) provides a useful reference for expected request/response formats.

#### 14.3.4 AWS SDK for Rust KMS Tests

The `aws-sdk-rust` project contains KMS integration tests:

- **Repository**: https://github.com/awslabs/aws-sdk-rust
- **Location**: `sdk/kms/tests/integration.rs`
- **Coverage**: Basic URI configuration and request signing validation
- **Value**: Validates that our responses are parseable by the official Rust SDK

### 14.4 AWS CLI Smoke Tests

```bash
# Create key
aws kms create-key --description "test key" \
    --endpoint-url http://localhost:4566

# Encrypt
aws kms encrypt --key-id <key-id> --plaintext "hello world" \
    --endpoint-url http://localhost:4566

# Decrypt
aws kms decrypt --ciphertext-blob fileb://ciphertext.bin \
    --endpoint-url http://localhost:4566

# Generate data key
aws kms generate-data-key --key-id <key-id> --key-spec AES_256 \
    --endpoint-url http://localhost:4566

# Create alias
aws kms create-alias --alias-name alias/my-key --target-key-id <key-id> \
    --endpoint-url http://localhost:4566

# List keys
aws kms list-keys --endpoint-url http://localhost:4566

# List aliases
aws kms list-aliases --endpoint-url http://localhost:4566
```

### 14.5 Terraform Integration Test

```hcl
provider "aws" {
  region                      = "us-east-1"
  access_key                  = "test"
  secret_key                  = "test"
  skip_credentials_validation = true
  skip_metadata_api_check     = true
  skip_requesting_account_id  = true

  endpoints {
    kms = "http://localhost:4566"
  }
}

resource "aws_kms_key" "test" {
  description = "Test KMS key"
}

resource "aws_kms_alias" "test" {
  name          = "alias/test-key"
  target_key_id = aws_kms_key.test.key_id
}
```

---

## 15. Phased Implementation Plan

### Phase 0: MVP -- Key Management + Symmetric Encryption + Aliases (16 Operations)

**Goal:** Core key lifecycle, symmetric encrypt/decrypt, envelope encryption (GenerateDataKey), alias management, tags, and GenerateRandom. Covers AWS CLI, Terraform `aws_kms_key`/`aws_kms_alias`, S3 SSE-KMS, and Secrets Manager integration.

**Estimated effort:** 5-7 days.

#### Step 0.1: Codegen

- Download KMS Smithy model, place at `codegen/smithy-model/kms.json`
- Add KMS service config to codegen with blob type support
- Generate `rustack-kms-model` crate with base64 blob serde
- Verify generated types compile and serde round-trip

#### Step 0.2: Cryptographic Engine

- Implement `CryptoKeyMaterial` enum with `Symmetric` variant
- Implement `generate_key_material` for `SYMMETRIC_DEFAULT`
- Implement ciphertext blob serialize/deserialize
- Implement `symmetric_encrypt` and `symmetric_decrypt` with AES-256-GCM
- Implement encryption context serialization
- Implement `GenerateRandom` using `aws_lc_rs::rand`
- Unit tests for all crypto operations

#### Step 0.3: Storage Engine

- Implement `KmsStore` with `DashMap` for keys, aliases, grants
- Implement `KmsKey` with metadata and state machine
- Implement key ID resolution (UUID, ARN, alias name, alias ARN)
- Implement alias storage and reserved alias auto-creation
- Implement tag storage with validation

#### Step 0.4: HTTP Layer

- Implement `KmsRouter` (`TrentService.*` dispatch)
- Implement `KmsHttpService` (reuse SSM's JSON protocol pattern)
- Implement `KmsServiceRouter` for gateway integration
- Wire into gateway with feature gate

#### Step 0.5: Core Operations (16)

- Key management: `CreateKey`, `DescribeKey`, `ListKeys`, `EnableKey`, `DisableKey`, `ScheduleKeyDeletion`, `CancelKeyDeletion`, `UpdateKeyDescription`
- Symmetric encryption: `Encrypt`, `Decrypt`, `ReEncrypt`
- Envelope encryption: `GenerateDataKey`, `GenerateDataKeyWithoutPlaintext`
- Aliases: `CreateAlias`, `DeleteAlias`, `ListAliases`, `UpdateAlias`
- Tags: `TagResource`, `UntagResource`, `ListResourceTags`
- Random: `GenerateRandom`

#### Step 0.6: Testing

- Unit tests for storage, crypto, validation
- Integration tests with `aws-sdk-kms`
- AWS CLI smoke tests
- Run LocalStack test suite, track pass/fail baseline

### Phase 1: Asymmetric Keys + Signing + HMAC (12 Operations)

**Goal:** Full cryptographic operation support. Enables JWT signing, document signing, HMAC-based authentication.

**Estimated effort:** 3-5 days.

- Extend `generate_key_material` for RSA, ECC, and HMAC key specs
- Implement RSA encryption/decryption (OAEP SHA-1/SHA-256)
- Implement RSA signing (PSS, PKCS1v1.5) and ECDSA signing
- Implement signature verification
- Implement HMAC generation and verification
- Add `Sign`, `Verify`, `GetPublicKey`
- Add `GenerateMac`, `VerifyMac`
- Add `GenerateDataKeyPair`, `GenerateDataKeyPairWithoutPlaintext`
- Add `GetKeyPolicy`, `PutKeyPolicy`, `ListKeyPolicies`
- Port LocalStack sign/verify and HMAC tests

### Phase 2: Grants + Key Rotation (6 Operations)

**Goal:** Full grant management and key rotation status tracking.

**Estimated effort:** 2-3 days.

- Implement grant storage and CRUD
- Add `CreateGrant`, `ListGrants`, `RetireGrant`, `RevokeGrant`, `ListRetirableGrants`
- Implement `EnableKeyRotation`, `DisableKeyRotation`, `GetKeyRotationStatus`
- Port LocalStack grant and rotation tests

---

## 16. Risk Analysis

### 16.1 Technical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `aws-lc-rs` build complexity (C/C++ dependency) | Medium | High | `aws-lc-rs` has good cargo build support; test on CI early. Fall back to pure-Rust `ring` + `rsa` crates if needed. |
| Ciphertext blob format incompatibility | Low | High | Our blobs are only consumed by our own Decrypt. Format is internal and self-consistent. AWS SDKs treat blobs as opaque bytes. |
| RSA key generation slowness | Medium | Low | RSA-4096 keygen takes ~1-2 seconds. Acceptable for local dev. Cache per-spec key material if needed. |
| ECC secp256k1 not in aws-lc-rs | High | Medium | Use `k256` crate from RustCrypto for secp256k1 only. Adds one dependency. |
| P-521 support incomplete in aws-lc-rs | Medium | Low | P-521 is rarely used. Defer to Phase 1 and validate library support. |
| Docker image size increase from aws-lc-rs | Medium | Medium | aws-lc-rs adds ~2-5 MB to binary. Monitor image size. |
| Blob field serde in codegen | Medium | Medium | KMS is the first service with blob types. Must extend codegen to handle base64 encoding. |
| Smithy model acquisition | Low | Low | KMS model is available at `aws/aws-models`. Same approach as DynamoDB/SSM. |

### 16.2 Scope Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Users expect key import (EXTERNAL origin) | Medium | Medium | Return structured error for GetParametersForImport; add in future phase |
| Users expect multi-region keys | Low | Low | Document as non-goal; return error for ReplicateKey |
| Users expect on-demand key rotation | Low | Low | Store rotation status but do not rotate; add RotateKeyOnDemand later |
| Feature creep (Secrets Manager, ACM integration) | Medium | Medium | Strict non-goals boundary; each service is independent |
| LocalStack test suite expects custom features (_custom_id_, _custom_key_material_) | Medium | Low | Skip `@markers.aws.only_localstack` tests; prefer AWS-correct behavior |

### 16.3 Behavioral Differences from AWS

| Behavior | AWS | Rustack | Justification |
|----------|-----|-----------|---------------|
| Ciphertext blob format | Proprietary internal format | Custom header + AES-GCM output | Blobs are opaque; SDKs never parse them |
| Key material storage | HSM-backed | In-memory | Local dev; no persistence requirement |
| Key deletion | Actually deletes after waiting period | Immediately transitions to PendingDeletion, blocks operations | Simpler; no background deletion scheduler |
| Grant authorization | Enforced in IAM | Stored but not enforced | No IAM engine; grants are for storage only |
| Key policies | Enforced | Stored but not enforced | No IAM engine |
| Key rotation | Actually rotates key material | Stores rotation flag only | Does not produce new key material on schedule |
| Rate limiting | 5,500 - 30,000 requests/second | Unlimited | Not meaningful for local dev |
| Cross-account access | Supported via grants/policies | Supported via multi-account routing | rustack-core handles account isolation |
| FIPS compliance | FIPS 140-2 Level 2 HSMs | No FIPS certification | Local dev only |

### 16.4 Implementation Effort Comparison

| Component | KMS Estimate | SSM | DynamoDB | Ratio (vs SSM) |
|-----------|-------------|-----|----------|----------------|
| Model (codegen) | ~3,000 | ~1,200 | ~4,000 | 2.5x |
| HTTP routing | ~100 | ~100 | ~100 | 1.0x |
| Crypto engine | ~1,500 | 0 | 0 | New |
| Storage engine | ~800 | ~500 | ~2,500 | 1.6x |
| Business logic | ~2,500 | ~1,200 | ~6,000 | 2.1x |
| **Total** | **~7,900** | **~3,000** | **~15,400** | **2.6x** |

KMS is approximately 2.6x the implementation effort of SSM, with the cryptographic engine being the major new component. The protocol layer is entirely reused. The storage engine is comparable in complexity to SSM (flat hashmap with metadata).
