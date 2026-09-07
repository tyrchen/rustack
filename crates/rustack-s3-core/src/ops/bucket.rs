//! Bucket CRUD operation handlers.
//!
//! Implements `create_bucket`, `delete_bucket`, `head_bucket`, `list_buckets`,
//! and `get_bucket_location`.

use rustack_s3_model::{
    error::S3Error,
    input::{
        CreateBucketInput, DeleteBucketInput, GetBucketLocationInput, HeadBucketInput,
        ListBucketsInput,
    },
    output::{CreateBucketOutput, GetBucketLocationOutput, HeadBucketOutput, ListBucketsOutput},
    types::{Bucket, BucketLocationConstraint, LocationType, Owner},
};
use tracing::debug;

use crate::{
    error::S3ServiceError, provider::RustackS3, state::object::Owner as InternalOwner,
    validation::validate_bucket_name,
};

/// Convert our internal [`InternalOwner`] to the model [`Owner`] type.
pub(crate) fn to_model_owner(owner: &InternalOwner) -> Owner {
    Owner {
        display_name: Some(owner.display_name.clone()),
        id: Some(owner.id.clone()),
    }
}

// Keep handlers lazy and uniformly awaitable at the dispatch boundary, including
// in-memory operations that currently complete without yielding.
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
impl RustackS3 {
    /// Create a new S3 bucket.
    pub async fn handle_create_bucket(
        &self,
        input: CreateBucketInput,
    ) -> Result<CreateBucketOutput, S3Error> {
        let bucket_name = input.bucket;

        validate_bucket_name(&bucket_name).map_err(S3ServiceError::into_s3_error)?;

        let region = input
            .create_bucket_configuration
            .and_then(|c| c.location_constraint)
            .map_or_else(
                || self.config.default_region.clone(),
                |lc: BucketLocationConstraint| lc.as_str().to_owned(),
            );

        let owner = InternalOwner::default();

        // Check if object lock is requested.
        let object_lock_enabled = input.object_lock_enabled_for_bucket.unwrap_or(false);

        self.state
            .create_bucket(bucket_name.clone(), region, owner)
            .map_err(S3ServiceError::into_s3_error)?;

        // If object lock was requested, enable it on the bucket.
        if object_lock_enabled {
            if let Ok(bucket) = self.state.get_bucket(&bucket_name) {
                *bucket.object_lock_enabled.write() = true;
                // Object lock requires versioning.
                bucket.enable_versioning();
            }
        }

        debug!(bucket = %bucket_name, "create_bucket completed");

        Ok(CreateBucketOutput {
            bucket_arn: None,
            location: Some(format!("/{bucket_name}")),
        })
    }

    /// Delete an S3 bucket.
    pub async fn handle_delete_bucket(&self, input: DeleteBucketInput) -> Result<(), S3Error> {
        let bucket_name = input.bucket;

        // Clean up CORS rules for this bucket.
        self.cors_index.delete_rules(&bucket_name);

        // Delete storage data for this bucket.
        self.storage.delete_bucket_data(&bucket_name);

        // Delete the bucket from state.
        self.state
            .delete_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        debug!(bucket = %bucket_name, "delete_bucket completed");

        Ok(())
    }

    /// Check if a bucket exists and is accessible (HEAD Bucket).
    pub async fn handle_head_bucket(
        &self,
        input: HeadBucketInput,
    ) -> Result<HeadBucketOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        Ok(HeadBucketOutput {
            access_point_alias: None,
            bucket_arn: None,
            bucket_location_name: Some(bucket.region.clone()),
            bucket_location_type: Some(LocationType::from("Region")),
            bucket_region: Some(bucket.region.clone()),
        })
    }

    /// List all buckets.
    pub async fn handle_list_buckets(
        &self,
        _input: ListBucketsInput,
    ) -> Result<ListBucketsOutput, S3Error> {
        let bucket_list = self.state.list_buckets();

        let buckets: Vec<Bucket> = bucket_list
            .into_iter()
            .map(|(name, creation_date)| Bucket {
                bucket_arn: None,
                name: Some(name),
                creation_date: Some(creation_date),
                bucket_region: None,
            })
            .collect();

        let owner = to_model_owner(&InternalOwner::default());

        Ok(ListBucketsOutput {
            buckets,
            continuation_token: None,
            owner: Some(owner),
            prefix: None,
        })
    }

    /// Get the location (region) of a bucket.
    pub async fn handle_get_bucket_location(
        &self,
        input: GetBucketLocationInput,
    ) -> Result<GetBucketLocationOutput, S3Error> {
        let bucket_name = input.bucket;

        let bucket = self
            .state
            .get_bucket(&bucket_name)
            .map_err(S3ServiceError::into_s3_error)?;

        let location_constraint = if bucket.region == "us-east-1" {
            // AWS returns null/empty for us-east-1.
            None
        } else {
            Some(BucketLocationConstraint::from(bucket.region.as_str()))
        };

        Ok(GetBucketLocationOutput {
            location_constraint,
        })
    }
}
