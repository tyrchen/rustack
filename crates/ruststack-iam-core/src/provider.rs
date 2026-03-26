//! Main IAM provider implementing all ~60 operations across 4 phases.
//!
//! Each operation method parses form parameters and returns an XML body
//! string plus a request ID. The handler bridges these to HTTP responses.
//!
//! # Phases
//!
//! - **Phase 0**: Users, roles, managed policies, policy attachment, access keys
//! - **Phase 1**: Groups, instance profiles, group membership
//! - **Phase 2**: Policy versions, inline policies
//! - **Phase 3**: Tagging, service-linked roles, simulation stubs, authorization details
#![allow(clippy::too_many_lines)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use dashmap::mapref::entry::Entry;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use ruststack_iam_http::{
    request::{
        get_optional_bool, get_optional_i32, get_optional_param, get_required_param,
        parse_string_list, parse_tag_list,
    },
    response::XmlWriter,
};
use ruststack_iam_model::error::IamError;
use tracing::debug;

use crate::{
    arn::iam_arn,
    config::IamConfig,
    id_gen::{generate_access_key_id, generate_iam_id, generate_secret_access_key},
    store::IamStore,
    types::{
        AccessKeyRecord, GroupRecord, InstanceProfileRecord, ManagedPolicyRecord,
        OidcProviderRecord, PolicyVersionRecord, RoleRecord, UserRecord,
    },
    validation::{
        validate_entity_name, validate_max_session_duration, validate_path,
        validate_policy_document,
    },
};

/// Characters that must be percent-encoded in policy document output.
const ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Maximum number of managed policies that can be attached to an entity.
const MAX_ATTACHED_POLICIES: usize = 20;

/// Maximum number of access keys per user.
const MAX_ACCESS_KEYS_PER_USER: usize = 2;

/// Maximum number of roles per instance profile.
const MAX_ROLES_PER_INSTANCE_PROFILE: usize = 1;

/// Maximum number of tags per entity.
const MAX_TAGS_PER_ENTITY: usize = 50;

/// Maximum number of versions per managed policy.
const MAX_POLICY_VERSIONS: usize = 5;

/// Default max items for list operations.
const DEFAULT_MAX_ITEMS: usize = 100;

/// Absolute maximum for `MaxItems` parameter.
const MAX_MAX_ITEMS: usize = 1000;

/// Main IAM provider holding all in-memory state.
#[derive(Debug)]
pub struct RustStackIam {
    store: Arc<IamStore>,
    config: Arc<IamConfig>,
}

impl RustStackIam {
    /// Create a new IAM provider.
    #[must_use]
    pub fn new(store: Arc<IamStore>, config: Arc<IamConfig>) -> Self {
        Self { store, config }
    }

    /// Get a reference to the underlying store.
    #[must_use]
    pub fn store(&self) -> &IamStore {
        &self.store
    }

    /// Get a reference to the configuration.
    #[must_use]
    pub fn config(&self) -> &IamConfig {
        &self.config
    }
}

// ---------------------------------------------------------------------------
// Helper: ISO 8601 timestamp
// ---------------------------------------------------------------------------

fn now_iso8601() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ---------------------------------------------------------------------------
// Helper: pagination
// ---------------------------------------------------------------------------

/// Parse pagination parameters (Marker, MaxItems).
fn parse_pagination(params: &[(String, String)]) -> (Option<usize>, usize) {
    let marker = get_optional_param(params, "Marker").and_then(|m| m.parse::<usize>().ok());
    let max_items = get_optional_i32(params, "MaxItems").map_or(DEFAULT_MAX_ITEMS, |v| {
        usize::try_from(v.max(0))
            .unwrap_or(0)
            .clamp(1, MAX_MAX_ITEMS)
    });
    (marker, max_items)
}

/// Apply pagination to a sorted list, returning (page, is_truncated, next_marker).
fn paginate<T>(
    items: &[T],
    marker: Option<usize>,
    max_items: usize,
) -> (&[T], bool, Option<String>) {
    let start = marker.unwrap_or(0);
    let end = (start + max_items).min(items.len());
    let page = if start < items.len() {
        &items[start..end]
    } else {
        &[]
    };
    let is_truncated = end < items.len();
    let next_marker = if is_truncated {
        Some(end.to_string())
    } else {
        None
    };
    (page, is_truncated, next_marker)
}

// ---------------------------------------------------------------------------
// Helper: URL-encode a policy document for XML output
// ---------------------------------------------------------------------------

fn url_encode_policy(doc: &str) -> String {
    utf8_percent_encode(doc, ENCODE_SET).to_string()
}

// ---------------------------------------------------------------------------
// Helper: XML tag writing
// ---------------------------------------------------------------------------

fn write_tags_xml(w: &mut XmlWriter, tags: &[(String, String)]) {
    w.start_element("Tags");
    for (key, value) in tags {
        w.start_element("member");
        w.write_element("Key", key);
        w.write_element("Value", value);
        w.end_element("member");
    }
    w.end_element("Tags");
}

fn write_user_xml(w: &mut XmlWriter, user: &UserRecord) {
    w.write_element("Path", &user.path);
    w.write_element("UserName", &user.user_name);
    w.write_element("UserId", &user.user_id);
    w.write_element("Arn", &user.arn);
    w.write_element("CreateDate", &user.create_date);
    if let Some(ref pb) = user.permissions_boundary {
        w.start_element("PermissionsBoundary");
        w.write_element("PermissionsBoundaryType", "Policy");
        w.write_element("PermissionsBoundaryArn", pb);
        w.end_element("PermissionsBoundary");
    }
    if !user.tags.is_empty() {
        write_tags_xml(w, &user.tags);
    }
}

fn write_role_xml(w: &mut XmlWriter, role: &RoleRecord) {
    w.write_element("Path", &role.path);
    w.write_element("RoleName", &role.role_name);
    w.write_element("RoleId", &role.role_id);
    w.write_element("Arn", &role.arn);
    w.write_element("CreateDate", &role.create_date);
    w.write_element(
        "AssumeRolePolicyDocument",
        &url_encode_policy(&role.assume_role_policy_document),
    );
    w.write_optional_element("Description", role.description.as_deref());
    w.write_i32_element("MaxSessionDuration", role.max_session_duration);
    if let Some(ref pb) = role.permissions_boundary {
        w.start_element("PermissionsBoundary");
        w.write_element("PermissionsBoundaryType", "Policy");
        w.write_element("PermissionsBoundaryArn", pb);
        w.end_element("PermissionsBoundary");
    }
    if !role.tags.is_empty() {
        write_tags_xml(w, &role.tags);
    }
}

fn write_group_xml(w: &mut XmlWriter, group: &GroupRecord) {
    w.write_element("Path", &group.path);
    w.write_element("GroupName", &group.group_name);
    w.write_element("GroupId", &group.group_id);
    w.write_element("Arn", &group.arn);
    w.write_element("CreateDate", &group.create_date);
}

fn write_policy_xml(w: &mut XmlWriter, policy: &ManagedPolicyRecord) {
    w.write_element("PolicyName", &policy.policy_name);
    w.write_element("PolicyId", &policy.policy_id);
    w.write_element("Arn", &policy.arn);
    w.write_element("Path", &policy.path);
    w.write_element("DefaultVersionId", &policy.default_version_id);
    w.write_i32_element("AttachmentCount", policy.attachment_count);
    w.write_i32_element(
        "PermissionsBoundaryUsageCount",
        policy.permissions_boundary_usage_count,
    );
    w.write_bool_element("IsAttachable", policy.is_attachable);
    w.write_optional_element("Description", policy.description.as_deref());
    w.write_element("CreateDate", &policy.create_date);
    w.write_element("UpdateDate", &policy.update_date);
    if !policy.tags.is_empty() {
        write_tags_xml(w, &policy.tags);
    }
}

/// Write instance profile XML with pre-fetched role records to avoid nested locks.
fn write_instance_profile_xml(w: &mut XmlWriter, ip: &InstanceProfileRecord, roles: &[RoleRecord]) {
    w.write_element("InstanceProfileName", &ip.instance_profile_name);
    w.write_element("InstanceProfileId", &ip.instance_profile_id);
    w.write_element("Arn", &ip.arn);
    w.write_element("Path", &ip.path);
    w.start_element("Roles");
    for role in roles {
        w.start_element("member");
        write_role_xml(w, role);
        w.end_element("member");
    }
    w.end_element("Roles");
    w.write_element("CreateDate", &ip.create_date);
    if !ip.tags.is_empty() {
        write_tags_xml(w, &ip.tags);
    }
}

/// Fetch role records for an instance profile's role names from the store.
fn fetch_roles_for_instance_profile(store: &IamStore, role_names: &[String]) -> Vec<RoleRecord> {
    role_names
        .iter()
        .filter_map(|name| store.roles.get(name).map(|r| r.value().clone()))
        .collect()
}

fn write_policy_version_xml(w: &mut XmlWriter, v: &PolicyVersionRecord) {
    w.write_element("Document", &url_encode_policy(&v.document));
    w.write_element("VersionId", &v.version_id);
    w.write_bool_element("IsDefaultVersion", v.is_default_version);
    w.write_element("CreateDate", &v.create_date);
}

// ---------------------------------------------------------------------------
// Helper: wrap a no-result response
// ---------------------------------------------------------------------------

fn empty_response(operation: &str, request_id: &str) -> String {
    let mut w = XmlWriter::new();
    w.start_response(operation);
    w.write_response_metadata(request_id);
    w.end_element(&format!("{operation}Response"));
    w.into_string()
}

// ============================================================================
// Phase 0 operations
// ============================================================================

impl RustStackIam {
    // ---- Users ----

    /// Create a new IAM user.
    pub fn create_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let path = get_optional_param(params, "Path").unwrap_or("/");
        let tags = parse_tag_list(params);
        let permissions_boundary =
            get_optional_param(params, "PermissionsBoundary").map(str::to_owned);

        validate_entity_name(user_name, 64)?;
        validate_path(path)?;
        if tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        let user = UserRecord {
            user_name: user_name.to_owned(),
            user_id: generate_iam_id("AIDA"),
            arn: iam_arn(&self.config.account_id, "user", path, user_name),
            path: path.to_owned(),
            create_date: now_iso8601(),
            tags,
            permissions_boundary,
            attached_policies: HashSet::new(),
            inline_policies: HashMap::new(),
            groups: HashSet::new(),
        };

        debug!(user_name, "creating IAM user");

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut w = XmlWriter::new();
        w.start_response("CreateUser");
        w.start_result("CreateUser");
        w.start_element("User");
        write_user_xml(&mut w, &user);
        w.end_element("User");
        w.end_element("CreateUserResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateUserResponse");

        match self.store.users.entry(user_name.to_owned()) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "User with name {user_name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(user);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Get an IAM user.
    pub fn get_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let user_name = get_optional_param(params, "UserName");

        let mut w = XmlWriter::new();
        w.start_response("GetUser");
        w.start_result("GetUser");
        w.start_element("User");

        if let Some(name) = user_name {
            let user = self.store.users.get(name).ok_or_else(|| {
                IamError::no_such_entity(format!("The user with name {name} cannot be found."))
            })?;
            write_user_xml(&mut w, &user);
        } else {
            // Return a default root user when no UserName is provided.
            w.write_element("Path", "/");
            w.write_element("UserName", "root");
            w.write_element("UserId", &self.config.account_id);
            w.write_element(
                "Arn",
                &format!("arn:aws:iam::{}:root", self.config.account_id),
            );
            w.write_element("CreateDate", "2024-01-01T00:00:00Z");
        }

        w.end_element("User");
        w.end_element("GetUserResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetUserResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an IAM user.
    pub fn delete_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone validation data out, then drop the guard before checking other maps.
        let (has_attached, has_inline, has_groups) = {
            let user = self.store.users.get(user_name).ok_or_else(|| {
                IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
            })?;
            (
                !user.attached_policies.is_empty(),
                !user.inline_policies.is_empty(),
                !user.groups.is_empty(),
            )
        };

        // Check for delete conflicts (guard already dropped).
        if has_attached {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must detach all policies first.",
            ));
        }
        if has_inline {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must delete all inline policies first.",
            ));
        }
        if has_groups {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must remove from all groups first.",
            ));
        }
        // Check for access keys (no guard held).
        let has_keys = self
            .store
            .access_keys
            .iter()
            .any(|entry| entry.value().user_name == user_name);
        if has_keys {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must delete all access keys first.",
            ));
        }

        debug!(user_name, "deleting IAM user");
        self.store.users.remove(user_name);

        Ok((empty_response("DeleteUser", &request_id), request_id))
    }

    /// List IAM users.
    pub fn list_users(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);

        let mut users: Vec<UserRecord> = self
            .store
            .users
            .iter()
            .filter(|e| e.value().path.starts_with(path_prefix))
            .map(|e| e.value().clone())
            .collect();
        users.sort_by(|a, b| a.user_name.cmp(&b.user_name));

        let (page, is_truncated, next_marker) = paginate(&users, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListUsers");
        w.start_result("ListUsers");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("Users");
        for user in page {
            w.start_element("member");
            write_user_xml(&mut w, user);
            w.end_element("member");
        }
        w.end_element("Users");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListUsersResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListUsersResponse");

        Ok((w.into_string(), request_id))
    }

    /// Update an IAM user (rename / change path).
    pub fn update_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let new_user_name = get_optional_param(params, "NewUserName");
        let new_path = get_optional_param(params, "NewPath");
        let request_id = uuid::Uuid::new_v4().to_string();

        if let Some(new_name) = new_user_name {
            validate_entity_name(new_name, 64)?;
            if new_name != user_name && self.store.users.contains_key(new_name) {
                return Err(IamError::entity_already_exists(format!(
                    "User with name {new_name} already exists."
                )));
            }
        }
        if let Some(p) = new_path {
            validate_path(p)?;
        }

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        if let Some(p) = new_path {
            p.clone_into(&mut user.path);
            user.arn = iam_arn(&self.config.account_id, "user", p, &user.user_name);
        }

        // Handle rename: need to re-key the map.
        let needs_rename = new_user_name.is_some_and(|n| n != user_name);
        if let Some(new_name) = new_user_name {
            new_name.clone_into(&mut user.user_name);
            let path = user.path.clone();
            user.arn = iam_arn(&self.config.account_id, "user", &path, new_name);
        }

        drop(user);

        if needs_rename {
            let new_name = new_user_name
                .ok_or_else(|| IamError::internal_error("Unexpected missing new user name"))?;
            if let Some((_, record)) = self.store.users.remove(user_name) {
                // Update group memberships.
                for group_name in &record.groups {
                    if let Some(mut grp) = self.store.groups.get_mut(group_name) {
                        grp.members.remove(user_name);
                        grp.members.insert(new_name.to_owned());
                    }
                }
                // Update access keys.
                for mut entry in self.store.access_keys.iter_mut() {
                    if entry.value().user_name == user_name {
                        new_name.clone_into(&mut entry.value_mut().user_name);
                    }
                }
                self.store.users.insert(new_name.to_owned(), record);
            }
        }

        debug!(user_name, "updated IAM user");
        Ok((empty_response("UpdateUser", &request_id), request_id))
    }

    // ---- Roles ----

    /// Create a new IAM role.
    pub fn create_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let assume_role_policy = get_required_param(params, "AssumeRolePolicyDocument")?;
        let path = get_optional_param(params, "Path").unwrap_or("/");
        let description = get_optional_param(params, "Description").map(str::to_owned);
        let max_session_duration = get_optional_i32(params, "MaxSessionDuration").unwrap_or(3600);
        let tags = parse_tag_list(params);
        let permissions_boundary =
            get_optional_param(params, "PermissionsBoundary").map(str::to_owned);

        validate_entity_name(role_name, 64)?;
        validate_path(path)?;
        validate_policy_document(assume_role_policy)?;
        validate_max_session_duration(max_session_duration)?;
        if tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        let role = RoleRecord {
            role_name: role_name.to_owned(),
            role_id: generate_iam_id("AROA"),
            arn: iam_arn(&self.config.account_id, "role", path, role_name),
            path: path.to_owned(),
            assume_role_policy_document: assume_role_policy.to_owned(),
            description,
            max_session_duration,
            create_date: now_iso8601(),
            tags,
            permissions_boundary,
            attached_policies: HashSet::new(),
            inline_policies: HashMap::new(),
            is_service_linked: false,
        };

        debug!(role_name, "creating IAM role");

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut w = XmlWriter::new();
        w.start_response("CreateRole");
        w.start_result("CreateRole");
        w.start_element("Role");
        write_role_xml(&mut w, &role);
        w.end_element("Role");
        w.end_element("CreateRoleResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateRoleResponse");

        match self.store.roles.entry(role_name.to_owned()) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "Role with name {role_name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(role);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Get an IAM role.
    pub fn get_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let role = self.store.roles.get(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetRole");
        w.start_result("GetRole");
        w.start_element("Role");
        write_role_xml(&mut w, &role);
        w.end_element("Role");
        w.end_element("GetRoleResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetRoleResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an IAM role.
    pub fn delete_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone validation data out, then drop the guard before checking other maps.
        let (has_attached, has_inline) = {
            let role = self.store.roles.get(role_name).ok_or_else(|| {
                IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
            })?;
            (
                !role.attached_policies.is_empty(),
                !role.inline_policies.is_empty(),
            )
        };

        // Check for delete conflicts (guard already dropped).
        if has_attached {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must detach all policies first.",
            ));
        }
        if has_inline {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must delete all inline policies first.",
            ));
        }
        // Check instance profiles (no guard held).
        let in_ip = self
            .store
            .instance_profiles
            .iter()
            .any(|e| e.value().roles.contains(&role_name.to_owned()));
        if in_ip {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must remove role from all instance profiles first.",
            ));
        }

        debug!(role_name, "deleting IAM role");
        self.store.roles.remove(role_name);

        Ok((empty_response("DeleteRole", &request_id), request_id))
    }

    /// List IAM roles.
    pub fn list_roles(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);

        let mut roles: Vec<RoleRecord> = self
            .store
            .roles
            .iter()
            .filter(|e| e.value().path.starts_with(path_prefix))
            .map(|e| e.value().clone())
            .collect();
        roles.sort_by(|a, b| a.role_name.cmp(&b.role_name));

        let (page, is_truncated, next_marker) = paginate(&roles, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListRoles");
        w.start_result("ListRoles");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("Roles");
        for role in page {
            w.start_element("member");
            write_role_xml(&mut w, role);
            w.end_element("member");
        }
        w.end_element("Roles");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListRolesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListRolesResponse");

        Ok((w.into_string(), request_id))
    }

    /// Update an IAM role.
    pub fn update_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let description = get_optional_param(params, "Description");
        let max_session_duration = get_optional_i32(params, "MaxSessionDuration");
        let request_id = uuid::Uuid::new_v4().to_string();

        if let Some(d) = max_session_duration {
            validate_max_session_duration(d)?;
        }

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        if let Some(desc) = description {
            role.description = Some(desc.to_owned());
        }
        if let Some(d) = max_session_duration {
            role.max_session_duration = d;
        }

        debug!(role_name, "updated IAM role");
        Ok((empty_response("UpdateRole", &request_id), request_id))
    }

    // ---- Managed Policies ----

    /// Create a managed policy.
    pub fn create_policy(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let policy_name = get_required_param(params, "PolicyName")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let path = get_optional_param(params, "Path").unwrap_or("/");
        let description = get_optional_param(params, "Description").map(str::to_owned);
        let tags = parse_tag_list(params);

        validate_entity_name(policy_name, 128)?;
        validate_path(path)?;
        validate_policy_document(policy_document)?;
        if tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        let arn = iam_arn(&self.config.account_id, "policy", path, policy_name);

        let now = now_iso8601();
        let version = PolicyVersionRecord {
            version_id: "v1".to_owned(),
            document: policy_document.to_owned(),
            is_default_version: true,
            create_date: now.clone(),
        };

        let policy = ManagedPolicyRecord {
            policy_name: policy_name.to_owned(),
            policy_id: generate_iam_id("ANPA"),
            arn: arn.clone(),
            path: path.to_owned(),
            description,
            create_date: now.clone(),
            update_date: now,
            is_attachable: true,
            attachment_count: 0,
            permissions_boundary_usage_count: 0,
            versions: vec![version],
            default_version_id: "v1".to_owned(),
            tags,
        };

        debug!(policy_name, "creating managed policy");

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut w = XmlWriter::new();
        w.start_response("CreatePolicy");
        w.start_result("CreatePolicy");
        w.start_element("Policy");
        write_policy_xml(&mut w, &policy);
        w.end_element("Policy");
        w.end_element("CreatePolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreatePolicyResponse");

        match self.store.policies.entry(arn) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "A policy called {policy_name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(policy);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Get a managed policy.
    pub fn get_policy(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let policy = self.store.policies.get(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetPolicy");
        w.start_result("GetPolicy");
        w.start_element("Policy");
        write_policy_xml(&mut w, &policy);
        w.end_element("Policy");
        w.end_element("GetPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete a managed policy.
    pub fn delete_policy(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let policy = self.store.policies.get(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        if policy.attachment_count > 0 {
            return Err(IamError::delete_conflict(
                "Cannot delete a policy attached to entities.",
            ));
        }

        drop(policy);

        debug!(policy_arn, "deleting managed policy");
        self.store.policies.remove(policy_arn);

        Ok((empty_response("DeletePolicy", &request_id), request_id))
    }

    /// List managed policies.
    pub fn list_policies(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let scope = get_optional_param(params, "Scope").unwrap_or("All");
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let only_attached = get_optional_bool(params, "OnlyAttached").unwrap_or(false);
        let _policy_usage_filter = get_optional_param(params, "PolicyUsageFilter");
        let (marker, max_items) = parse_pagination(params);

        let mut policies: Vec<ManagedPolicyRecord> = self
            .store
            .policies
            .iter()
            .filter(|e| {
                let p = e.value();
                // Path prefix filter.
                if !p.path.starts_with(path_prefix) {
                    return false;
                }
                // Scope filter.
                let is_aws_managed = p.arn.starts_with("arn:aws:iam::aws:");
                if scope == "AWS" && !is_aws_managed {
                    return false;
                }
                if scope == "Local" && is_aws_managed {
                    return false;
                }
                // OnlyAttached filter.
                if only_attached && p.attachment_count == 0 {
                    return false;
                }
                true
            })
            .map(|e| e.value().clone())
            .collect();
        policies.sort_by(|a, b| a.policy_name.cmp(&b.policy_name));

        let (page, is_truncated, next_marker) = paginate(&policies, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListPolicies");
        w.start_result("ListPolicies");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("Policies");
        for pol in page {
            w.start_element("member");
            write_policy_xml(&mut w, pol);
            w.end_element("member");
        }
        w.end_element("Policies");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListPoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListPoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- User Policy Attachment ----

    /// Attach a managed policy to a user.
    pub fn attach_user_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Verify both exist with read-only guards (dropped immediately).
        if !self.store.users.contains_key(user_name) {
            return Err(IamError::no_such_entity(format!(
                "The user with name {user_name} cannot be found."
            )));
        }
        if !self.store.policies.contains_key(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} does not exist."
            )));
        }

        // Acquire write lock on user only.
        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        if user.attached_policies.len() >= MAX_ATTACHED_POLICIES
            && !user.attached_policies.contains(policy_arn)
        {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_ATTACHED_POLICIES} attached policies per entity"
            )));
        }

        let inserted = user.attached_policies.insert(policy_arn.to_owned());
        drop(user); // Drop user lock before acquiring policy lock.

        if inserted {
            if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
                pol.attachment_count += 1;
            }
        }

        debug!(user_name, policy_arn, "attached policy to user");
        Ok((empty_response("AttachUserPolicy", &request_id), request_id))
    }

    /// Detach a managed policy from a user.
    pub fn detach_user_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        if !user.attached_policies.remove(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} is not attached to user {user_name}."
            )));
        }

        drop(user); // Drop user lock before acquiring policy lock.

        if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
            pol.attachment_count = (pol.attachment_count - 1).max(0);
        }

        debug!(user_name, policy_arn, "detached policy from user");
        Ok((empty_response("DetachUserPolicy", &request_id), request_id))
    }

    /// List managed policies attached to a user.
    pub fn list_attached_user_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone attached policy ARNs, then drop the user guard.
        let attached_policy_arns: Vec<String> = {
            let user = self.store.users.get(user_name).ok_or_else(|| {
                IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
            })?;
            user.attached_policies.iter().cloned().collect()
        };

        // Guard dropped, now safe to query the policies map.
        let mut policies: Vec<(String, String)> = attached_policy_arns
            .iter()
            .filter_map(|arn| {
                let pol = self.store.policies.get(arn)?;
                if pol.path.starts_with(path_prefix) {
                    Some((pol.policy_name.clone(), arn.clone()))
                } else {
                    None
                }
            })
            .collect();
        policies.sort_by(|a, b| a.0.cmp(&b.0));

        let (page, is_truncated, next_marker) = paginate(&policies, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListAttachedUserPolicies");
        w.start_result("ListAttachedUserPolicies");
        w.start_element("AttachedPolicies");
        for (name, arn) in page {
            w.start_element("member");
            w.write_element("PolicyName", name);
            w.write_element("PolicyArn", arn);
            w.end_element("member");
        }
        w.end_element("AttachedPolicies");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListAttachedUserPoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListAttachedUserPoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Role Policy Attachment ----

    /// Attach a managed policy to a role.
    pub fn attach_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Verify both exist with read-only guards (dropped immediately).
        if !self.store.roles.contains_key(role_name) {
            return Err(IamError::no_such_entity(format!(
                "The role with name {role_name} cannot be found."
            )));
        }
        if !self.store.policies.contains_key(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} does not exist."
            )));
        }

        // Acquire write lock on role only.
        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        if role.attached_policies.len() >= MAX_ATTACHED_POLICIES
            && !role.attached_policies.contains(policy_arn)
        {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_ATTACHED_POLICIES} attached policies per entity"
            )));
        }

        let inserted = role.attached_policies.insert(policy_arn.to_owned());
        drop(role); // Drop role lock before acquiring policy lock.

        if inserted {
            if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
                pol.attachment_count += 1;
            }
        }

        debug!(role_name, policy_arn, "attached policy to role");
        Ok((empty_response("AttachRolePolicy", &request_id), request_id))
    }

    /// Detach a managed policy from a role.
    pub fn detach_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        if !role.attached_policies.remove(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} is not attached to role {role_name}."
            )));
        }

        drop(role); // Drop role lock before acquiring policy lock.

        if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
            pol.attachment_count = (pol.attachment_count - 1).max(0);
        }

        debug!(role_name, policy_arn, "detached policy from role");
        Ok((empty_response("DetachRolePolicy", &request_id), request_id))
    }

    /// List managed policies attached to a role.
    pub fn list_attached_role_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone attached policy ARNs, then drop the role guard.
        let attached_policy_arns: Vec<String> = {
            let role = self.store.roles.get(role_name).ok_or_else(|| {
                IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
            })?;
            role.attached_policies.iter().cloned().collect()
        };

        // Guard dropped, now safe to query the policies map.
        let mut policies: Vec<(String, String)> = attached_policy_arns
            .iter()
            .filter_map(|arn| {
                let pol = self.store.policies.get(arn)?;
                if pol.path.starts_with(path_prefix) {
                    Some((pol.policy_name.clone(), arn.clone()))
                } else {
                    None
                }
            })
            .collect();
        policies.sort_by(|a, b| a.0.cmp(&b.0));

        let (page, is_truncated, next_marker) = paginate(&policies, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListAttachedRolePolicies");
        w.start_result("ListAttachedRolePolicies");
        w.start_element("AttachedPolicies");
        for (name, arn) in page {
            w.start_element("member");
            w.write_element("PolicyName", name);
            w.write_element("PolicyArn", arn);
            w.end_element("member");
        }
        w.end_element("AttachedPolicies");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListAttachedRolePoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListAttachedRolePoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Access Keys ----

    /// Create an access key for a user.
    pub fn create_access_key(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_optional_param(params, "UserName").unwrap_or("root");
        let request_id = uuid::Uuid::new_v4().to_string();

        // Verify user exists (unless root).
        if user_name != "root" && !self.store.users.contains_key(user_name) {
            return Err(IamError::no_such_entity(format!(
                "The user with name {user_name} cannot be found."
            )));
        }

        // Check limit.
        let existing_count = self
            .store
            .access_keys
            .iter()
            .filter(|e| e.value().user_name == user_name)
            .count();
        if existing_count >= MAX_ACCESS_KEYS_PER_USER {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_ACCESS_KEYS_PER_USER} access keys per user"
            )));
        }

        let key = AccessKeyRecord {
            access_key_id: generate_access_key_id(),
            secret_access_key: generate_secret_access_key(),
            user_name: user_name.to_owned(),
            status: "Active".to_owned(),
            create_date: now_iso8601(),
        };

        debug!(user_name, access_key_id = %key.access_key_id, "creating access key");

        let mut w = XmlWriter::new();
        w.start_response("CreateAccessKey");
        w.start_result("CreateAccessKey");
        w.start_element("AccessKey");
        w.write_element("UserName", &key.user_name);
        w.write_element("AccessKeyId", &key.access_key_id);
        w.write_element("Status", &key.status);
        w.write_element("SecretAccessKey", &key.secret_access_key);
        w.write_element("CreateDate", &key.create_date);
        w.end_element("AccessKey");
        w.end_element("CreateAccessKeyResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateAccessKeyResponse");

        self.store
            .access_keys
            .insert(key.access_key_id.clone(), key);

        Ok((w.into_string(), request_id))
    }

    /// Delete an access key.
    pub fn delete_access_key(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let access_key_id = get_required_param(params, "AccessKeyId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        if self.store.access_keys.remove(access_key_id).is_none() {
            return Err(IamError::no_such_entity(format!(
                "The Access Key with id {access_key_id} cannot be found."
            )));
        }

        debug!(access_key_id, "deleted access key");
        Ok((empty_response("DeleteAccessKey", &request_id), request_id))
    }

    /// List access keys for a user.
    pub fn list_access_keys(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_optional_param(params, "UserName").unwrap_or("root");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut keys: Vec<AccessKeyRecord> = self
            .store
            .access_keys
            .iter()
            .filter(|e| e.value().user_name == user_name)
            .map(|e| e.value().clone())
            .collect();
        keys.sort_by(|a, b| a.access_key_id.cmp(&b.access_key_id));

        let (page, is_truncated, next_marker) = paginate(&keys, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListAccessKeys");
        w.start_result("ListAccessKeys");
        w.start_element("AccessKeyMetadata");
        for key in page {
            w.start_element("member");
            w.write_element("UserName", &key.user_name);
            w.write_element("AccessKeyId", &key.access_key_id);
            w.write_element("Status", &key.status);
            w.write_element("CreateDate", &key.create_date);
            w.end_element("member");
        }
        w.end_element("AccessKeyMetadata");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListAccessKeysResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListAccessKeysResponse");

        Ok((w.into_string(), request_id))
    }

    /// Update access key status.
    pub fn update_access_key(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let access_key_id = get_required_param(params, "AccessKeyId")?;
        let status = get_required_param(params, "Status")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        if status != "Active" && status != "Inactive" {
            return Err(IamError::invalid_input(format!(
                "Invalid status: {status}. Must be Active or Inactive."
            )));
        }

        let mut key = self
            .store
            .access_keys
            .get_mut(access_key_id)
            .ok_or_else(|| {
                IamError::no_such_entity(format!(
                    "The Access Key with id {access_key_id} cannot be found."
                ))
            })?;

        status.clone_into(&mut key.status);

        debug!(access_key_id, status, "updated access key status");
        Ok((empty_response("UpdateAccessKey", &request_id), request_id))
    }

    /// Get access key last used info.
    pub fn get_access_key_last_used(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let access_key_id = get_required_param(params, "AccessKeyId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let key = self.store.access_keys.get(access_key_id).ok_or_else(|| {
            IamError::no_such_entity(format!(
                "The Access Key with id {access_key_id} cannot be found."
            ))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetAccessKeyLastUsed");
        w.start_result("GetAccessKeyLastUsed");
        w.write_element("UserName", &key.user_name);
        w.start_element("AccessKeyLastUsed");
        // Stub: always return "N/A" since we don't track usage.
        w.write_element("Region", "N/A");
        w.write_element("ServiceName", "N/A");
        w.write_element("LastUsedDate", "N/A");
        w.end_element("AccessKeyLastUsed");
        w.end_element("GetAccessKeyLastUsedResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetAccessKeyLastUsedResponse");

        Ok((w.into_string(), request_id))
    }
}

// ============================================================================
// Phase 1 operations
// ============================================================================

impl RustStackIam {
    // ---- Groups ----

    /// Create a new IAM group.
    pub fn create_group(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let path = get_optional_param(params, "Path").unwrap_or("/");

        validate_entity_name(group_name, 128)?;
        validate_path(path)?;

        let group = GroupRecord {
            group_name: group_name.to_owned(),
            group_id: generate_iam_id("AGPA"),
            arn: iam_arn(&self.config.account_id, "group", path, group_name),
            path: path.to_owned(),
            create_date: now_iso8601(),
            attached_policies: HashSet::new(),
            inline_policies: HashMap::new(),
            members: HashSet::new(),
        };

        debug!(group_name, "creating IAM group");

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut w = XmlWriter::new();
        w.start_response("CreateGroup");
        w.start_result("CreateGroup");
        w.start_element("Group");
        write_group_xml(&mut w, &group);
        w.end_element("Group");
        w.end_element("CreateGroupResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateGroupResponse");

        match self.store.groups.entry(group_name.to_owned()) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "Group with name {group_name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(group);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Get an IAM group (includes member details).
    pub fn get_group(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone the group data and member names, then drop the guard.
        let (group_clone, member_names): (GroupRecord, Vec<String>) = {
            let group = self.store.groups.get(group_name).ok_or_else(|| {
                IamError::no_such_entity(format!(
                    "The group with name {group_name} cannot be found."
                ))
            })?;
            (group.clone(), group.members.iter().cloned().collect())
        };

        // Guard dropped, now safe to query the users map.
        let mut member_users: Vec<UserRecord> = member_names
            .iter()
            .filter_map(|name| self.store.users.get(name).map(|u| u.value().clone()))
            .collect();
        member_users.sort_by(|a, b| a.user_name.cmp(&b.user_name));

        let (page, is_truncated, next_marker) = paginate(&member_users, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("GetGroup");
        w.start_result("GetGroup");
        w.start_element("Group");
        write_group_xml(&mut w, &group_clone);
        w.end_element("Group");
        w.start_element("Users");
        for user in page {
            w.start_element("member");
            write_user_xml(&mut w, user);
            w.end_element("member");
        }
        w.end_element("Users");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("GetGroupResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetGroupResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an IAM group.
    pub fn delete_group(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let group = self.store.groups.get(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if !group.attached_policies.is_empty() {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must detach all policies first.",
            ));
        }
        if !group.inline_policies.is_empty() {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must delete all inline policies first.",
            ));
        }
        if !group.members.is_empty() {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must remove all members first.",
            ));
        }

        drop(group);

        debug!(group_name, "deleting IAM group");
        self.store.groups.remove(group_name);

        Ok((empty_response("DeleteGroup", &request_id), request_id))
    }

    /// List IAM groups.
    pub fn list_groups(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);

        let mut groups: Vec<GroupRecord> = self
            .store
            .groups
            .iter()
            .filter(|e| e.value().path.starts_with(path_prefix))
            .map(|e| e.value().clone())
            .collect();
        groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));

        let (page, is_truncated, next_marker) = paginate(&groups, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListGroups");
        w.start_result("ListGroups");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("Groups");
        for group in page {
            w.start_element("member");
            write_group_xml(&mut w, group);
            w.end_element("member");
        }
        w.end_element("Groups");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListGroupsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListGroupsResponse");

        Ok((w.into_string(), request_id))
    }

    /// Update an IAM group (rename / change path).
    pub fn update_group(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let new_group_name = get_optional_param(params, "NewGroupName");
        let new_path = get_optional_param(params, "NewPath");
        let request_id = uuid::Uuid::new_v4().to_string();

        if let Some(new_name) = new_group_name {
            validate_entity_name(new_name, 128)?;
            if new_name != group_name && self.store.groups.contains_key(new_name) {
                return Err(IamError::entity_already_exists(format!(
                    "Group with name {new_name} already exists."
                )));
            }
        }
        if let Some(p) = new_path {
            validate_path(p)?;
        }

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if let Some(p) = new_path {
            p.clone_into(&mut group.path);
            group.arn = iam_arn(&self.config.account_id, "group", p, &group.group_name);
        }

        let needs_rename = new_group_name.is_some_and(|n| n != group_name);
        if let Some(new_name) = new_group_name {
            new_name.clone_into(&mut group.group_name);
            let path = group.path.clone();
            group.arn = iam_arn(&self.config.account_id, "group", &path, new_name);
        }

        drop(group);

        if needs_rename {
            let new_name = new_group_name
                .ok_or_else(|| IamError::internal_error("Unexpected missing new group name"))?;
            if let Some((_, record)) = self.store.groups.remove(group_name) {
                // Update user group memberships.
                for member in &record.members {
                    if let Some(mut user) = self.store.users.get_mut(member) {
                        user.groups.remove(group_name);
                        user.groups.insert(new_name.to_owned());
                    }
                }
                self.store.groups.insert(new_name.to_owned(), record);
            }
        }

        debug!(group_name, "updated IAM group");
        Ok((empty_response("UpdateGroup", &request_id), request_id))
    }

    // ---- Group Membership ----

    /// Add a user to a group.
    pub fn add_user_to_group(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let user_name = get_required_param(params, "UserName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        if !self.store.users.contains_key(user_name) {
            return Err(IamError::no_such_entity(format!(
                "The user with name {user_name} cannot be found."
            )));
        }

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        group.members.insert(user_name.to_owned());
        drop(group);

        if let Some(mut user) = self.store.users.get_mut(user_name) {
            user.groups.insert(group_name.to_owned());
        }

        debug!(user_name, group_name, "added user to group");
        Ok((empty_response("AddUserToGroup", &request_id), request_id))
    }

    /// Remove a user from a group.
    pub fn remove_user_from_group(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let user_name = get_required_param(params, "UserName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if !group.members.remove(user_name) {
            return Err(IamError::no_such_entity(format!(
                "User {user_name} is not in group {group_name}."
            )));
        }
        drop(group);

        if let Some(mut user) = self.store.users.get_mut(user_name) {
            user.groups.remove(group_name);
        }

        debug!(user_name, group_name, "removed user from group");
        Ok((
            empty_response("RemoveUserFromGroup", &request_id),
            request_id,
        ))
    }

    /// List groups that a user belongs to.
    pub fn list_groups_for_user(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone group names, then drop the user guard.
        let group_names: Vec<String> = {
            let user = self.store.users.get(user_name).ok_or_else(|| {
                IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
            })?;
            user.groups.iter().cloned().collect()
        };

        // Guard dropped, now safe to query the groups map.
        let mut groups: Vec<GroupRecord> = group_names
            .iter()
            .filter_map(|name| self.store.groups.get(name).map(|g| g.value().clone()))
            .collect();
        groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));

        let (page, is_truncated, next_marker) = paginate(&groups, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListGroupsForUser");
        w.start_result("ListGroupsForUser");
        w.start_element("Groups");
        for group in page {
            w.start_element("member");
            write_group_xml(&mut w, group);
            w.end_element("member");
        }
        w.end_element("Groups");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListGroupsForUserResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListGroupsForUserResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Group Policy Attachment ----

    /// Attach a managed policy to a group.
    pub fn attach_group_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Verify both exist with read-only guards (dropped immediately).
        if !self.store.groups.contains_key(group_name) {
            return Err(IamError::no_such_entity(format!(
                "The group with name {group_name} cannot be found."
            )));
        }
        if !self.store.policies.contains_key(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} does not exist."
            )));
        }

        // Acquire write lock on group only.
        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if group.attached_policies.len() >= MAX_ATTACHED_POLICIES
            && !group.attached_policies.contains(policy_arn)
        {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_ATTACHED_POLICIES} attached policies per entity"
            )));
        }

        let inserted = group.attached_policies.insert(policy_arn.to_owned());
        drop(group); // Drop group lock before acquiring policy lock.

        if inserted {
            if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
                pol.attachment_count += 1;
            }
        }

        debug!(group_name, policy_arn, "attached policy to group");
        Ok((empty_response("AttachGroupPolicy", &request_id), request_id))
    }

    /// Detach a managed policy from a group.
    pub fn detach_group_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if !group.attached_policies.remove(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} is not attached to group {group_name}."
            )));
        }

        drop(group); // Drop group lock before acquiring policy lock.

        if let Some(mut pol) = self.store.policies.get_mut(policy_arn) {
            pol.attachment_count = (pol.attachment_count - 1).max(0);
        }

        debug!(group_name, policy_arn, "detached policy from group");
        Ok((empty_response("DetachGroupPolicy", &request_id), request_id))
    }

    /// List managed policies attached to a group.
    pub fn list_attached_group_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone attached policy ARNs, then drop the group guard.
        let attached_policy_arns: Vec<String> = {
            let group = self.store.groups.get(group_name).ok_or_else(|| {
                IamError::no_such_entity(format!(
                    "The group with name {group_name} cannot be found."
                ))
            })?;
            group.attached_policies.iter().cloned().collect()
        };

        // Guard dropped, now safe to query the policies map.
        let mut policies: Vec<(String, String)> = attached_policy_arns
            .iter()
            .filter_map(|arn| {
                let pol = self.store.policies.get(arn)?;
                if pol.path.starts_with(path_prefix) {
                    Some((pol.policy_name.clone(), arn.clone()))
                } else {
                    None
                }
            })
            .collect();
        policies.sort_by(|a, b| a.0.cmp(&b.0));

        let (page, is_truncated, next_marker) = paginate(&policies, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListAttachedGroupPolicies");
        w.start_result("ListAttachedGroupPolicies");
        w.start_element("AttachedPolicies");
        for (name, arn) in page {
            w.start_element("member");
            w.write_element("PolicyName", name);
            w.write_element("PolicyArn", arn);
            w.end_element("member");
        }
        w.end_element("AttachedPolicies");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListAttachedGroupPoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListAttachedGroupPoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Instance Profiles ----

    /// Create an instance profile.
    pub fn create_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let name = get_required_param(params, "InstanceProfileName")?;
        let path = get_optional_param(params, "Path").unwrap_or("/");
        let tags = parse_tag_list(params);

        validate_entity_name(name, 128)?;
        validate_path(path)?;
        if tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        let ip = InstanceProfileRecord {
            instance_profile_name: name.to_owned(),
            instance_profile_id: generate_iam_id("AIPA"),
            arn: iam_arn(&self.config.account_id, "instance-profile", path, name),
            path: path.to_owned(),
            create_date: now_iso8601(),
            tags,
            roles: Vec::new(),
        };

        debug!(name, "creating instance profile");

        let request_id = uuid::Uuid::new_v4().to_string();
        let mut w = XmlWriter::new();
        w.start_response("CreateInstanceProfile");
        w.start_result("CreateInstanceProfile");
        w.start_element("InstanceProfile");
        // No roles yet, so no nested lock issue with empty roles list.
        write_instance_profile_xml(&mut w, &ip, &[]);
        w.end_element("InstanceProfile");
        w.end_element("CreateInstanceProfileResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateInstanceProfileResponse");

        match self.store.instance_profiles.entry(name.to_owned()) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "Instance Profile {name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(ip);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Get an instance profile.
    pub fn get_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let name = get_required_param(params, "InstanceProfileName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Clone IP data and drop guard before fetching roles from another map.
        let ip = {
            let ip_ref = self.store.instance_profiles.get(name).ok_or_else(|| {
                IamError::no_such_entity(format!("Instance Profile {name} cannot be found."))
            })?;
            ip_ref.clone()
        };

        let roles = fetch_roles_for_instance_profile(&self.store, &ip.roles);

        let mut w = XmlWriter::new();
        w.start_response("GetInstanceProfile");
        w.start_result("GetInstanceProfile");
        w.start_element("InstanceProfile");
        write_instance_profile_xml(&mut w, &ip, &roles);
        w.end_element("InstanceProfile");
        w.end_element("GetInstanceProfileResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetInstanceProfileResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an instance profile.
    pub fn delete_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let name = get_required_param(params, "InstanceProfileName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let ip = self.store.instance_profiles.get(name).ok_or_else(|| {
            IamError::no_such_entity(format!("Instance Profile {name} cannot be found."))
        })?;

        if !ip.roles.is_empty() {
            return Err(IamError::delete_conflict(
                "Cannot delete entity, must remove all roles first.",
            ));
        }

        drop(ip);

        debug!(name, "deleting instance profile");
        self.store.instance_profiles.remove(name);

        Ok((
            empty_response("DeleteInstanceProfile", &request_id),
            request_id,
        ))
    }

    /// List instance profiles.
    pub fn list_instance_profiles(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let path_prefix = get_optional_param(params, "PathPrefix").unwrap_or("/");
        let (marker, max_items) = parse_pagination(params);

        let mut profiles: Vec<InstanceProfileRecord> = self
            .store
            .instance_profiles
            .iter()
            .filter(|e| e.value().path.starts_with(path_prefix))
            .map(|e| e.value().clone())
            .collect();
        profiles.sort_by(|a, b| a.instance_profile_name.cmp(&b.instance_profile_name));

        let (page, is_truncated, next_marker) = paginate(&profiles, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListInstanceProfiles");
        w.start_result("ListInstanceProfiles");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("InstanceProfiles");
        for ip in page {
            let roles = fetch_roles_for_instance_profile(&self.store, &ip.roles);
            w.start_element("member");
            write_instance_profile_xml(&mut w, ip, &roles);
            w.end_element("member");
        }
        w.end_element("InstanceProfiles");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListInstanceProfilesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListInstanceProfilesResponse");

        Ok((w.into_string(), request_id))
    }

    /// List instance profiles that contain a specific role.
    pub fn list_instance_profiles_for_role(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        if !self.store.roles.contains_key(role_name) {
            return Err(IamError::no_such_entity(format!(
                "The role with name {role_name} cannot be found."
            )));
        }

        let mut profiles: Vec<InstanceProfileRecord> = self
            .store
            .instance_profiles
            .iter()
            .filter(|e| e.value().roles.contains(&role_name.to_owned()))
            .map(|e| e.value().clone())
            .collect();
        profiles.sort_by(|a, b| a.instance_profile_name.cmp(&b.instance_profile_name));

        let (page, is_truncated, next_marker) = paginate(&profiles, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListInstanceProfilesForRole");
        w.start_result("ListInstanceProfilesForRole");
        w.write_bool_element("IsTruncated", is_truncated);
        w.start_element("InstanceProfiles");
        for ip in page {
            let roles = fetch_roles_for_instance_profile(&self.store, &ip.roles);
            w.start_element("member");
            write_instance_profile_xml(&mut w, ip, &roles);
            w.end_element("member");
        }
        w.end_element("InstanceProfiles");
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListInstanceProfilesForRoleResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListInstanceProfilesForRoleResponse");

        Ok((w.into_string(), request_id))
    }

    /// Add a role to an instance profile.
    pub fn add_role_to_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let ip_name = get_required_param(params, "InstanceProfileName")?;
        let role_name = get_required_param(params, "RoleName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        if !self.store.roles.contains_key(role_name) {
            return Err(IamError::no_such_entity(format!(
                "The role with name {role_name} cannot be found."
            )));
        }

        let mut ip = self
            .store
            .instance_profiles
            .get_mut(ip_name)
            .ok_or_else(|| {
                IamError::no_such_entity(format!("Instance Profile {ip_name} cannot be found."))
            })?;

        if ip.roles.len() >= MAX_ROLES_PER_INSTANCE_PROFILE
            && !ip.roles.contains(&role_name.to_owned())
        {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_ROLES_PER_INSTANCE_PROFILE} roles per instance profile"
            )));
        }

        if !ip.roles.contains(&role_name.to_owned()) {
            ip.roles.push(role_name.to_owned());
        }

        debug!(ip_name, role_name, "added role to instance profile");
        Ok((
            empty_response("AddRoleToInstanceProfile", &request_id),
            request_id,
        ))
    }

    /// Remove a role from an instance profile.
    pub fn remove_role_from_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let ip_name = get_required_param(params, "InstanceProfileName")?;
        let role_name = get_required_param(params, "RoleName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut ip = self
            .store
            .instance_profiles
            .get_mut(ip_name)
            .ok_or_else(|| {
                IamError::no_such_entity(format!("Instance Profile {ip_name} cannot be found."))
            })?;

        let before_len = ip.roles.len();
        ip.roles.retain(|r| r != role_name);
        if ip.roles.len() == before_len {
            return Err(IamError::no_such_entity(format!(
                "Role {role_name} is not in instance profile {ip_name}."
            )));
        }

        debug!(ip_name, role_name, "removed role from instance profile");
        Ok((
            empty_response("RemoveRoleFromInstanceProfile", &request_id),
            request_id,
        ))
    }
}

// ============================================================================
// Phase 2 operations
// ============================================================================

impl RustStackIam {
    // ---- Policy Versions ----

    /// Create a new version of a managed policy.
    pub fn create_policy_version(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let set_as_default = get_optional_bool(params, "SetAsDefault").unwrap_or(false);
        let request_id = uuid::Uuid::new_v4().to_string();

        validate_policy_document(policy_document)?;

        let mut policy = self.store.policies.get_mut(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        if policy.versions.len() >= MAX_POLICY_VERSIONS {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_POLICY_VERSIONS} versions per policy. Delete a version first."
            )));
        }

        // Compute next version number.
        let max_ver: u32 = policy
            .versions
            .iter()
            .filter_map(|v| v.version_id.strip_prefix('v').and_then(|n| n.parse().ok()))
            .max()
            .unwrap_or(0);
        let new_version_id = format!("v{}", max_ver + 1);
        let now = now_iso8601();

        if set_as_default {
            for v in &mut policy.versions {
                v.is_default_version = false;
            }
            policy.default_version_id.clone_from(&new_version_id);
        }

        let version = PolicyVersionRecord {
            version_id: new_version_id,
            document: policy_document.to_owned(),
            is_default_version: set_as_default,
            create_date: now.clone(),
        };

        let mut w = XmlWriter::new();
        w.start_response("CreatePolicyVersion");
        w.start_result("CreatePolicyVersion");
        w.start_element("PolicyVersion");
        write_policy_version_xml(&mut w, &version);
        w.end_element("PolicyVersion");
        w.end_element("CreatePolicyVersionResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreatePolicyVersionResponse");

        policy.versions.push(version);
        policy.update_date = now;

        Ok((w.into_string(), request_id))
    }

    /// Get a specific version of a managed policy.
    pub fn get_policy_version(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let version_id = get_required_param(params, "VersionId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let policy = self.store.policies.get(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        let version = policy
            .versions
            .iter()
            .find(|v| v.version_id == version_id)
            .ok_or_else(|| {
                IamError::no_such_entity(format!("Policy version {version_id} does not exist."))
            })?;

        let mut w = XmlWriter::new();
        w.start_response("GetPolicyVersion");
        w.start_result("GetPolicyVersion");
        w.start_element("PolicyVersion");
        write_policy_version_xml(&mut w, version);
        w.end_element("PolicyVersion");
        w.end_element("GetPolicyVersionResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetPolicyVersionResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete a specific version of a managed policy.
    pub fn delete_policy_version(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let version_id = get_required_param(params, "VersionId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut policy = self.store.policies.get_mut(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        // Cannot delete the default version.
        if policy.default_version_id == version_id {
            return Err(IamError::delete_conflict(
                "Cannot delete the default policy version.",
            ));
        }

        let before = policy.versions.len();
        policy.versions.retain(|v| v.version_id != version_id);
        if policy.versions.len() == before {
            return Err(IamError::no_such_entity(format!(
                "Policy version {version_id} does not exist."
            )));
        }

        debug!(policy_arn, version_id, "deleted policy version");
        Ok((
            empty_response("DeletePolicyVersion", &request_id),
            request_id,
        ))
    }

    /// List all versions of a managed policy.
    pub fn list_policy_versions(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let policy = self.store.policies.get(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        let (page, is_truncated, next_marker) = paginate(&policy.versions, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListPolicyVersions");
        w.start_result("ListPolicyVersions");
        w.start_element("Versions");
        for v in page {
            w.start_element("member");
            write_policy_version_xml(&mut w, v);
            w.end_element("member");
        }
        w.end_element("Versions");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListPolicyVersionsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListPolicyVersionsResponse");

        Ok((w.into_string(), request_id))
    }

    /// Set the default version of a managed policy.
    pub fn set_default_policy_version(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let version_id = get_required_param(params, "VersionId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut policy = self.store.policies.get_mut(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        if !policy.versions.iter().any(|v| v.version_id == version_id) {
            return Err(IamError::no_such_entity(format!(
                "Policy version {version_id} does not exist."
            )));
        }

        for v in &mut policy.versions {
            v.is_default_version = v.version_id == version_id;
        }
        version_id.clone_into(&mut policy.default_version_id);

        debug!(policy_arn, version_id, "set default policy version");
        Ok((
            empty_response("SetDefaultPolicyVersion", &request_id),
            request_id,
        ))
    }

    // ---- Inline User Policies ----

    /// Put (create/update) an inline policy on a user.
    pub fn put_user_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        validate_entity_name(policy_name, 128)?;
        validate_policy_document(policy_document)?;

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        user.inline_policies
            .insert(policy_name.to_owned(), policy_document.to_owned());

        debug!(user_name, policy_name, "put inline user policy");
        Ok((empty_response("PutUserPolicy", &request_id), request_id))
    }

    /// Get an inline policy from a user.
    pub fn get_user_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let user = self.store.users.get(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        let document = user.inline_policies.get(policy_name).ok_or_else(|| {
            IamError::no_such_entity(format!(
                "The user policy with name {policy_name} cannot be found."
            ))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetUserPolicy");
        w.start_result("GetUserPolicy");
        w.write_element("UserName", user_name);
        w.write_element("PolicyName", policy_name);
        w.write_element("PolicyDocument", &url_encode_policy(document));
        w.end_element("GetUserPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetUserPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an inline policy from a user.
    pub fn delete_user_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        if user.inline_policies.remove(policy_name).is_none() {
            return Err(IamError::no_such_entity(format!(
                "The user policy with name {policy_name} cannot be found."
            )));
        }

        debug!(user_name, policy_name, "deleted inline user policy");
        Ok((empty_response("DeleteUserPolicy", &request_id), request_id))
    }

    /// List inline policy names for a user.
    pub fn list_user_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let user = self.store.users.get(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        let mut names: Vec<String> = user.inline_policies.keys().cloned().collect();
        names.sort();

        let (page, is_truncated, next_marker) = paginate(&names, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListUserPolicies");
        w.start_result("ListUserPolicies");
        w.start_element("PolicyNames");
        for name in page {
            w.write_element("member", name);
        }
        w.end_element("PolicyNames");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListUserPoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListUserPoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Inline Role Policies ----

    /// Put (create/update) an inline policy on a role.
    pub fn put_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        validate_entity_name(policy_name, 128)?;
        validate_policy_document(policy_document)?;

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        role.inline_policies
            .insert(policy_name.to_owned(), policy_document.to_owned());

        debug!(role_name, policy_name, "put inline role policy");
        Ok((empty_response("PutRolePolicy", &request_id), request_id))
    }

    /// Get an inline policy from a role.
    pub fn get_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let role = self.store.roles.get(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        let document = role.inline_policies.get(policy_name).ok_or_else(|| {
            IamError::no_such_entity(format!(
                "The role policy with name {policy_name} cannot be found."
            ))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetRolePolicy");
        w.start_result("GetRolePolicy");
        w.write_element("RoleName", role_name);
        w.write_element("PolicyName", policy_name);
        w.write_element("PolicyDocument", &url_encode_policy(document));
        w.end_element("GetRolePolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetRolePolicyResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an inline policy from a role.
    pub fn delete_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        if role.inline_policies.remove(policy_name).is_none() {
            return Err(IamError::no_such_entity(format!(
                "The role policy with name {policy_name} cannot be found."
            )));
        }

        debug!(role_name, policy_name, "deleted inline role policy");
        Ok((empty_response("DeleteRolePolicy", &request_id), request_id))
    }

    /// List inline policy names for a role.
    pub fn list_role_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let role = self.store.roles.get(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        let mut names: Vec<String> = role.inline_policies.keys().cloned().collect();
        names.sort();

        let (page, is_truncated, next_marker) = paginate(&names, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListRolePolicies");
        w.start_result("ListRolePolicies");
        w.start_element("PolicyNames");
        for name in page {
            w.write_element("member", name);
        }
        w.end_element("PolicyNames");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListRolePoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListRolePoliciesResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Inline Group Policies ----

    /// Put (create/update) an inline policy on a group.
    pub fn put_group_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        validate_entity_name(policy_name, 128)?;
        validate_policy_document(policy_document)?;

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        group
            .inline_policies
            .insert(policy_name.to_owned(), policy_document.to_owned());

        debug!(group_name, policy_name, "put inline group policy");
        Ok((empty_response("PutGroupPolicy", &request_id), request_id))
    }

    /// Get an inline policy from a group.
    pub fn get_group_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let group = self.store.groups.get(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        let document = group.inline_policies.get(policy_name).ok_or_else(|| {
            IamError::no_such_entity(format!(
                "The group policy with name {policy_name} cannot be found."
            ))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetGroupPolicy");
        w.start_result("GetGroupPolicy");
        w.write_element("GroupName", group_name);
        w.write_element("PolicyName", policy_name);
        w.write_element("PolicyDocument", &url_encode_policy(document));
        w.end_element("GetGroupPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetGroupPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an inline policy from a group.
    pub fn delete_group_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let policy_name = get_required_param(params, "PolicyName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut group = self.store.groups.get_mut(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        if group.inline_policies.remove(policy_name).is_none() {
            return Err(IamError::no_such_entity(format!(
                "The group policy with name {policy_name} cannot be found."
            )));
        }

        debug!(group_name, policy_name, "deleted inline group policy");
        Ok((empty_response("DeleteGroupPolicy", &request_id), request_id))
    }

    /// List inline policy names for a group.
    pub fn list_group_policies(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let group_name = get_required_param(params, "GroupName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let group = self.store.groups.get(group_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The group with name {group_name} cannot be found."))
        })?;

        let mut names: Vec<String> = group.inline_policies.keys().cloned().collect();
        names.sort();

        let (page, is_truncated, next_marker) = paginate(&names, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListGroupPolicies");
        w.start_result("ListGroupPolicies");
        w.start_element("PolicyNames");
        for name in page {
            w.write_element("member", name);
        }
        w.end_element("PolicyNames");
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListGroupPoliciesResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListGroupPoliciesResponse");

        Ok((w.into_string(), request_id))
    }
}

// ============================================================================
// Phase 3 operations
// ============================================================================

impl RustStackIam {
    // ---- User Tags ----

    /// Tag a user.
    pub fn tag_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let new_tags = parse_tag_list(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        // Calculate resulting tags BEFORE mutating.
        let mut merged_tags = user.tags.clone();
        for (key, value) in &new_tags {
            if let Some(existing) = merged_tags.iter_mut().find(|(k, _)| k == key) {
                existing.1.clone_from(value);
            } else {
                merged_tags.push((key.clone(), value.clone()));
            }
        }

        if merged_tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        // Now actually apply the merged tags.
        user.tags = merged_tags;

        debug!(user_name, count = new_tags.len(), "tagged user");
        Ok((empty_response("TagUser", &request_id), request_id))
    }

    /// Remove tags from a user.
    pub fn untag_user(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let tag_keys = parse_string_list(params, "TagKeys");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut user = self.store.users.get_mut(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        user.tags.retain(|(k, _)| !tag_keys.contains(k));

        debug!(user_name, count = tag_keys.len(), "untagged user");
        Ok((empty_response("UntagUser", &request_id), request_id))
    }

    /// List tags for a user.
    pub fn list_user_tags(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let user_name = get_required_param(params, "UserName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let user = self.store.users.get(user_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The user with name {user_name} cannot be found."))
        })?;

        let (page, is_truncated, next_marker) = paginate(&user.tags, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListUserTags");
        w.start_result("ListUserTags");
        write_tags_xml(&mut w, page);
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListUserTagsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListUserTagsResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Role Tags ----

    /// Tag a role.
    pub fn tag_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let new_tags = parse_tag_list(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        // Calculate resulting tags BEFORE mutating.
        let mut merged_tags = role.tags.clone();
        for (key, value) in &new_tags {
            if let Some(existing) = merged_tags.iter_mut().find(|(k, _)| k == key) {
                existing.1.clone_from(value);
            } else {
                merged_tags.push((key.clone(), value.clone()));
            }
        }

        if merged_tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        // Now actually apply the merged tags.
        role.tags = merged_tags;

        debug!(role_name, count = new_tags.len(), "tagged role");
        Ok((empty_response("TagRole", &request_id), request_id))
    }

    /// Remove tags from a role.
    pub fn untag_role(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let tag_keys = parse_string_list(params, "TagKeys");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        role.tags.retain(|(k, _)| !tag_keys.contains(k));

        debug!(role_name, count = tag_keys.len(), "untagged role");
        Ok((empty_response("UntagRole", &request_id), request_id))
    }

    /// List tags for a role.
    pub fn list_role_tags(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let role = self.store.roles.get(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        let (page, is_truncated, next_marker) = paginate(&role.tags, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListRoleTags");
        w.start_result("ListRoleTags");
        write_tags_xml(&mut w, page);
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListRoleTagsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListRoleTagsResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Service-Linked Roles ----

    /// Create a service-linked role.
    pub fn create_service_linked_role(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let service_name = get_required_param(params, "AWSServiceName")?;
        let description = get_optional_param(params, "Description").map(str::to_owned);
        let _custom_suffix = get_optional_param(params, "CustomSuffix");
        let request_id = uuid::Uuid::new_v4().to_string();

        // Derive role name from service name.
        // e.g., "elasticmapreduce.amazonaws.com" -> "AWSServiceRoleForElasticMapReduce"
        let service_prefix = service_name.split('.').next().unwrap_or(service_name);
        let capitalized = capitalize_service_name(service_prefix);
        let role_name = format!("AWSServiceRoleFor{capitalized}");

        let trust_policy = format!(
            r#"{{"Version":"2012-10-17","Statement":[{{"Effect":"Allow","Principal":{{"Service":"{service_name}"}},"Action":"sts:AssumeRole"}}]}}"#
        );

        let role = RoleRecord {
            role_name: role_name.clone(),
            role_id: generate_iam_id("AROA"),
            arn: iam_arn(
                &self.config.account_id,
                "role",
                "/aws-service-role/",
                &role_name,
            ),
            path: "/aws-service-role/".to_owned(),
            assume_role_policy_document: trust_policy,
            description,
            max_session_duration: 3600,
            create_date: now_iso8601(),
            tags: Vec::new(),
            permissions_boundary: None,
            attached_policies: HashSet::new(),
            inline_policies: HashMap::new(),
            is_service_linked: true,
        };

        debug!(role_name, service_name, "creating service-linked role");

        let mut w = XmlWriter::new();
        w.start_response("CreateServiceLinkedRole");
        w.start_result("CreateServiceLinkedRole");
        w.start_element("Role");
        write_role_xml(&mut w, &role);
        w.end_element("Role");
        w.end_element("CreateServiceLinkedRoleResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateServiceLinkedRoleResponse");

        match self.store.roles.entry(role_name.clone()) {
            Entry::Occupied(_) => {
                return Err(IamError::entity_already_exists(format!(
                    "Service role {role_name} already exists."
                )));
            }
            Entry::Vacant(e) => {
                e.insert(role);
            }
        }

        Ok((w.into_string(), request_id))
    }

    /// Delete a service-linked role.
    pub fn delete_service_linked_role(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let role = self.store.roles.get(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        if !role.is_service_linked {
            return Err(IamError::invalid_input(format!(
                "Role {role_name} is not a service-linked role."
            )));
        }

        drop(role);

        let deletion_task_id = uuid::Uuid::new_v4().to_string();

        debug!(role_name, deletion_task_id, "deleting service-linked role");
        self.store.roles.remove(role_name);

        let mut w = XmlWriter::new();
        w.start_response("DeleteServiceLinkedRole");
        w.start_result("DeleteServiceLinkedRole");
        w.write_element("DeletionTaskId", &deletion_task_id);
        w.end_element("DeleteServiceLinkedRoleResult");
        w.write_response_metadata(&request_id);
        w.end_element("DeleteServiceLinkedRoleResponse");

        Ok((w.into_string(), request_id))
    }

    /// Get the status of a service-linked role deletion task.
    pub fn get_service_linked_role_deletion_status(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let _deletion_task_id = get_required_param(params, "DeletionTaskId")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        // Always return SUCCEEDED since we delete immediately.
        let mut w = XmlWriter::new();
        w.start_response("GetServiceLinkedRoleDeletionStatus");
        w.start_result("GetServiceLinkedRoleDeletionStatus");
        w.write_element("Status", "SUCCEEDED");
        w.end_element("GetServiceLinkedRoleDeletionStatusResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetServiceLinkedRoleDeletionStatusResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Update Assume Role Policy ----

    /// Update the trust policy (assume role policy document) of a role.
    pub fn update_assume_role_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let role_name = get_required_param(params, "RoleName")?;
        let policy_document = get_required_param(params, "PolicyDocument")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        validate_policy_document(policy_document)?;

        let mut role = self.store.roles.get_mut(role_name).ok_or_else(|| {
            IamError::no_such_entity(format!("The role with name {role_name} cannot be found."))
        })?;

        policy_document.clone_into(&mut role.assume_role_policy_document);

        debug!(role_name, "updated assume role policy");
        Ok((
            empty_response("UpdateAssumeRolePolicy", &request_id),
            request_id,
        ))
    }

    // ---- Simulation Stubs ----

    /// Simulate a principal policy (stub: all allowed).
    pub fn simulate_principal_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let _policy_source_arn = get_required_param(params, "PolicySourceArn")?;
        let action_names = parse_string_list(params, "ActionNames");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut w = XmlWriter::new();
        w.start_response("SimulatePrincipalPolicy");
        w.start_result("SimulatePrincipalPolicy");
        w.start_element("EvaluationResults");
        for action in &action_names {
            w.start_element("member");
            w.write_element("EvalActionName", action);
            w.write_element("EvalDecision", "allowed");
            w.write_element("EvalResourceName", "*");
            w.end_element("member");
        }
        w.end_element("EvaluationResults");
        w.write_bool_element("IsTruncated", false);
        w.end_element("SimulatePrincipalPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("SimulatePrincipalPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    /// Simulate a custom policy (stub: all allowed).
    pub fn simulate_custom_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let action_names = parse_string_list(params, "ActionNames");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut w = XmlWriter::new();
        w.start_response("SimulateCustomPolicy");
        w.start_result("SimulateCustomPolicy");
        w.start_element("EvaluationResults");
        for action in &action_names {
            w.start_element("member");
            w.write_element("EvalActionName", action);
            w.write_element("EvalDecision", "allowed");
            w.write_element("EvalResourceName", "*");
            w.end_element("member");
        }
        w.end_element("EvaluationResults");
        w.write_bool_element("IsTruncated", false);
        w.end_element("SimulateCustomPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("SimulateCustomPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- List Entities For Policy ----

    /// List entities (users, groups, roles) that a managed policy is attached to.
    pub fn list_entities_for_policy(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let _entity_filter = get_optional_param(params, "EntityFilter");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        if !self.store.policies.contains_key(policy_arn) {
            return Err(IamError::no_such_entity(format!(
                "Policy {policy_arn} does not exist."
            )));
        }

        // Collect users with this policy attached.
        let mut policy_users: Vec<(String, String)> = Vec::new();
        for entry in &self.store.users {
            if entry.value().attached_policies.contains(policy_arn) {
                policy_users.push((
                    entry.value().user_name.clone(),
                    entry.value().user_id.clone(),
                ));
            }
        }
        policy_users.sort_by(|a, b| a.0.cmp(&b.0));

        // Collect groups with this policy attached.
        let mut policy_groups: Vec<(String, String)> = Vec::new();
        for entry in &self.store.groups {
            if entry.value().attached_policies.contains(policy_arn) {
                policy_groups.push((
                    entry.value().group_name.clone(),
                    entry.value().group_id.clone(),
                ));
            }
        }
        policy_groups.sort_by(|a, b| a.0.cmp(&b.0));

        // Collect roles with this policy attached.
        let mut policy_roles: Vec<(String, String)> = Vec::new();
        for entry in &self.store.roles {
            if entry.value().attached_policies.contains(policy_arn) {
                policy_roles.push((
                    entry.value().role_name.clone(),
                    entry.value().role_id.clone(),
                ));
            }
        }
        policy_roles.sort_by(|a, b| a.0.cmp(&b.0));

        // Combine all entities into a single list for pagination.
        // Each entry: (entity_type, name, id)
        let mut all_entities: Vec<(&str, &str, &str)> = Vec::new();
        for (name, id) in &policy_groups {
            all_entities.push(("group", name, id));
        }
        for (name, id) in &policy_users {
            all_entities.push(("user", name, id));
        }
        for (name, id) in &policy_roles {
            all_entities.push(("role", name, id));
        }

        let start = marker.unwrap_or(0);
        let end = (start + max_items).min(all_entities.len());
        let page = if start < all_entities.len() {
            &all_entities[start..end]
        } else {
            &[]
        };
        let is_truncated = end < all_entities.len();

        // Partition the page back into entity types.
        let page_groups: Vec<_> = page.iter().filter(|(t, _, _)| *t == "group").collect();
        let page_users: Vec<_> = page.iter().filter(|(t, _, _)| *t == "user").collect();
        let page_roles: Vec<_> = page.iter().filter(|(t, _, _)| *t == "role").collect();

        let mut w = XmlWriter::new();
        w.start_response("ListEntitiesForPolicy");
        w.start_result("ListEntitiesForPolicy");

        w.start_element("PolicyGroups");
        for (_, name, id) in &page_groups {
            w.start_element("member");
            w.write_element("GroupName", name);
            w.write_element("GroupId", id);
            w.end_element("member");
        }
        w.end_element("PolicyGroups");

        w.start_element("PolicyUsers");
        for (_, name, id) in &page_users {
            w.start_element("member");
            w.write_element("UserName", name);
            w.write_element("UserId", id);
            w.end_element("member");
        }
        w.end_element("PolicyUsers");

        w.start_element("PolicyRoles");
        for (_, name, id) in &page_roles {
            w.start_element("member");
            w.write_element("RoleName", name);
            w.write_element("RoleId", id);
            w.end_element("member");
        }
        w.end_element("PolicyRoles");

        w.write_bool_element("IsTruncated", is_truncated);
        if is_truncated {
            w.write_element("Marker", &end.to_string());
        }
        w.end_element("ListEntitiesForPolicyResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListEntitiesForPolicyResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Get Account Authorization Details ----

    /// Get comprehensive account authorization details.
    #[allow(clippy::too_many_lines)]
    pub fn get_account_authorization_details(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let filter_list = parse_string_list(params, "Filter");
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let include_users = filter_list.is_empty() || filter_list.iter().any(|f| f == "User");
        let include_groups = filter_list.is_empty() || filter_list.iter().any(|f| f == "Group");
        let include_roles = filter_list.is_empty() || filter_list.iter().any(|f| f == "Role");
        let include_local_policies =
            filter_list.is_empty() || filter_list.iter().any(|f| f == "LocalManagedPolicy");
        let include_aws_policies =
            filter_list.is_empty() || filter_list.iter().any(|f| f == "AWSManagedPolicy");

        let _ = (marker, max_items); // Full dump for simplicity.

        // Pre-fetch all data to avoid nested DashMap locks during XML generation.
        let policy_name_by_arn: HashMap<String, String> = self
            .store
            .policies
            .iter()
            .map(|e| (e.key().clone(), e.value().policy_name.clone()))
            .collect();

        let instance_profiles: Vec<InstanceProfileRecord> = self
            .store
            .instance_profiles
            .iter()
            .map(|e| e.value().clone())
            .collect();

        let mut w = XmlWriter::new();
        w.start_response("GetAccountAuthorizationDetails");
        w.start_result("GetAccountAuthorizationDetails");

        // Users
        if include_users {
            w.start_element("UserDetailList");
            let mut users: Vec<UserRecord> =
                self.store.users.iter().map(|e| e.value().clone()).collect();
            users.sort_by(|a, b| a.user_name.cmp(&b.user_name));
            for user in &users {
                w.start_element("member");
                w.write_element("Path", &user.path);
                w.write_element("UserName", &user.user_name);
                w.write_element("UserId", &user.user_id);
                w.write_element("Arn", &user.arn);
                w.write_element("CreateDate", &user.create_date);

                // Inline policies
                w.start_element("UserPolicyList");
                let mut inline_names: Vec<&String> = user.inline_policies.keys().collect();
                inline_names.sort();
                for name in inline_names {
                    w.start_element("member");
                    w.write_element("PolicyName", name);
                    if let Some(doc) = user.inline_policies.get(name) {
                        w.write_element("PolicyDocument", &url_encode_policy(doc));
                    }
                    w.end_element("member");
                }
                w.end_element("UserPolicyList");

                // Group memberships
                w.start_element("GroupList");
                let mut groups: Vec<&String> = user.groups.iter().collect();
                groups.sort();
                for g in groups {
                    w.write_element("member", g);
                }
                w.end_element("GroupList");

                // Attached managed policies
                w.start_element("AttachedManagedPolicies");
                let mut attached: Vec<&String> = user.attached_policies.iter().collect();
                attached.sort();
                for arn in attached {
                    w.start_element("member");
                    if let Some(policy_name) = policy_name_by_arn.get(arn.as_str()) {
                        w.write_element("PolicyName", policy_name);
                    }
                    w.write_element("PolicyArn", arn);
                    w.end_element("member");
                }
                w.end_element("AttachedManagedPolicies");

                if !user.tags.is_empty() {
                    write_tags_xml(&mut w, &user.tags);
                }
                w.end_element("member");
            }
            w.end_element("UserDetailList");
        }

        // Groups
        if include_groups {
            w.start_element("GroupDetailList");
            let mut groups: Vec<GroupRecord> = self
                .store
                .groups
                .iter()
                .map(|e| e.value().clone())
                .collect();
            groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));
            for group in &groups {
                w.start_element("member");
                w.write_element("Path", &group.path);
                w.write_element("GroupName", &group.group_name);
                w.write_element("GroupId", &group.group_id);
                w.write_element("Arn", &group.arn);
                w.write_element("CreateDate", &group.create_date);

                w.start_element("GroupPolicyList");
                let mut inline_names: Vec<&String> = group.inline_policies.keys().collect();
                inline_names.sort();
                for name in inline_names {
                    w.start_element("member");
                    w.write_element("PolicyName", name);
                    if let Some(doc) = group.inline_policies.get(name) {
                        w.write_element("PolicyDocument", &url_encode_policy(doc));
                    }
                    w.end_element("member");
                }
                w.end_element("GroupPolicyList");

                w.start_element("AttachedManagedPolicies");
                let mut attached: Vec<&String> = group.attached_policies.iter().collect();
                attached.sort();
                for arn in attached {
                    w.start_element("member");
                    if let Some(policy_name) = policy_name_by_arn.get(arn.as_str()) {
                        w.write_element("PolicyName", policy_name);
                    }
                    w.write_element("PolicyArn", arn);
                    w.end_element("member");
                }
                w.end_element("AttachedManagedPolicies");
                w.end_element("member");
            }
            w.end_element("GroupDetailList");
        }

        // Roles
        if include_roles {
            w.start_element("RoleDetailList");
            let mut roles: Vec<RoleRecord> =
                self.store.roles.iter().map(|e| e.value().clone()).collect();
            roles.sort_by(|a, b| a.role_name.cmp(&b.role_name));
            for role in &roles {
                w.start_element("member");
                w.write_element("Path", &role.path);
                w.write_element("RoleName", &role.role_name);
                w.write_element("RoleId", &role.role_id);
                w.write_element("Arn", &role.arn);
                w.write_element("CreateDate", &role.create_date);
                w.write_element(
                    "AssumeRolePolicyDocument",
                    &url_encode_policy(&role.assume_role_policy_document),
                );

                // Instance profiles for this role (using pre-fetched data).
                w.start_element("InstanceProfileList");
                for ip in &instance_profiles {
                    if ip.roles.contains(&role.role_name) {
                        // Roles are already cloned above, find matching ones.
                        let ip_roles: Vec<&RoleRecord> = roles
                            .iter()
                            .filter(|r| ip.roles.contains(&r.role_name))
                            .collect();
                        let ip_roles_owned: Vec<RoleRecord> =
                            ip_roles.into_iter().cloned().collect();
                        w.start_element("member");
                        write_instance_profile_xml(&mut w, ip, &ip_roles_owned);
                        w.end_element("member");
                    }
                }
                w.end_element("InstanceProfileList");

                w.start_element("RolePolicyList");
                let mut inline_names: Vec<&String> = role.inline_policies.keys().collect();
                inline_names.sort();
                for name in inline_names {
                    w.start_element("member");
                    w.write_element("PolicyName", name);
                    if let Some(doc) = role.inline_policies.get(name) {
                        w.write_element("PolicyDocument", &url_encode_policy(doc));
                    }
                    w.end_element("member");
                }
                w.end_element("RolePolicyList");

                w.start_element("AttachedManagedPolicies");
                let mut attached: Vec<&String> = role.attached_policies.iter().collect();
                attached.sort();
                for arn in attached {
                    w.start_element("member");
                    if let Some(policy_name) = policy_name_by_arn.get(arn.as_str()) {
                        w.write_element("PolicyName", policy_name);
                    }
                    w.write_element("PolicyArn", arn);
                    w.end_element("member");
                }
                w.end_element("AttachedManagedPolicies");

                if !role.tags.is_empty() {
                    write_tags_xml(&mut w, &role.tags);
                }
                w.end_element("member");
            }
            w.end_element("RoleDetailList");
        }

        // Policies
        if include_local_policies || include_aws_policies {
            w.start_element("Policies");
            let mut policies: Vec<ManagedPolicyRecord> = self
                .store
                .policies
                .iter()
                .map(|e| e.value().clone())
                .collect();
            policies.sort_by(|a, b| a.policy_name.cmp(&b.policy_name));
            for pol in &policies {
                let is_aws = pol.arn.starts_with("arn:aws:iam::aws:");
                if (is_aws && !include_aws_policies) || (!is_aws && !include_local_policies) {
                    continue;
                }
                w.start_element("member");
                w.write_element("PolicyName", &pol.policy_name);
                w.write_element("PolicyId", &pol.policy_id);
                w.write_element("Arn", &pol.arn);
                w.write_element("Path", &pol.path);
                w.write_element("DefaultVersionId", &pol.default_version_id);
                w.write_i32_element("AttachmentCount", pol.attachment_count);
                w.write_i32_element(
                    "PermissionsBoundaryUsageCount",
                    pol.permissions_boundary_usage_count,
                );
                w.write_bool_element("IsAttachable", pol.is_attachable);
                w.write_optional_element("Description", pol.description.as_deref());
                w.write_element("CreateDate", &pol.create_date);
                w.write_element("UpdateDate", &pol.update_date);
                w.start_element("PolicyVersionList");
                for v in &pol.versions {
                    w.start_element("member");
                    write_policy_version_xml(&mut w, v);
                    w.end_element("member");
                }
                w.end_element("PolicyVersionList");
                w.end_element("member");
            }
            w.end_element("Policies");
        }

        w.write_bool_element("IsTruncated", false);
        w.end_element("GetAccountAuthorizationDetailsResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetAccountAuthorizationDetailsResponse");

        Ok((w.into_string(), request_id))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Capitalize a service name for service-linked role naming.
///
/// E.g., `"elasticmapreduce"` -> `"ElasticMapReduce"`.
/// Simple heuristic: capitalize the first letter and any letter following known
/// word boundaries (camel-case common AWS service names).
fn capitalize_service_name(name: &str) -> String {
    let known_mappings: &[(&str, &str)] = &[
        ("elasticmapreduce", "ElasticMapReduce"),
        ("autoscaling", "AutoScaling"),
        ("elasticloadbalancing", "ElasticLoadBalancing"),
        ("cloudformation", "CloudFormation"),
        ("cloudwatch", "CloudWatch"),
        ("codedeploy", "CodeDeploy"),
        ("codecommit", "CodeCommit"),
        ("codepipeline", "CodePipeline"),
        ("dynamodb", "DynamoDB"),
        ("ec2", "EC2"),
        ("ecs", "ECS"),
        ("eks", "EKS"),
        ("elasticache", "ElastiCache"),
        ("guardduty", "GuardDuty"),
        ("lambda", "Lambda"),
        ("rds", "RDS"),
        ("redshift", "Redshift"),
        ("s3", "S3"),
        ("sns", "SNS"),
        ("sqs", "SQS"),
        ("ssm", "SSM"),
    ];

    let lower = name.to_lowercase();
    for (key, val) in known_mappings {
        if lower == *key {
            return (*val).to_owned();
        }
    }

    // Fallback: capitalize first letter.
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => {
            let mut result = first.to_uppercase().to_string();
            result.extend(chars);
            result
        }
        None => String::new(),
    }
}

// ============================================================================
// Phase 4 operations
// ============================================================================

impl RustStackIam {
    // ---- OIDC Providers ----

    /// Normalize an OIDC provider URL by stripping the `https://` scheme.
    fn normalize_oidc_url(url: &str) -> String {
        url.strip_prefix("https://")
            .or_else(|| url.strip_prefix("http://"))
            .unwrap_or(url)
            .trim_end_matches('/')
            .to_owned()
    }

    /// Create an OIDC identity provider.
    pub fn create_open_id_connect_provider(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let url = get_required_param(params, "Url")?;
        let client_id_list = parse_string_list(params, "ClientIDList");
        let thumbprint_list = parse_string_list(params, "ThumbprintList");
        let tags = parse_tag_list(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        if url.is_empty() {
            return Err(IamError::invalid_input("Url must not be empty"));
        }

        if thumbprint_list.is_empty() {
            return Err(IamError::invalid_input(
                "ThumbprintList must contain at least one thumbprint",
            ));
        }

        if tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        let normalized = Self::normalize_oidc_url(url);
        let arn = format!(
            "arn:aws:iam::{}:oidc-provider/{}",
            self.config.account_id, normalized
        );

        if self.store.oidc_providers.contains_key(&arn) {
            return Err(IamError::entity_already_exists(format!(
                "Provider with url {url} already exists."
            )));
        }

        let record = OidcProviderRecord {
            arn: arn.clone(),
            url: url.to_owned(),
            client_id_list,
            thumbprint_list,
            tags,
            create_date: now_iso8601(),
        };

        self.store.oidc_providers.insert(arn.clone(), record);

        debug!(url, arn, "created OIDC provider");

        let mut w = XmlWriter::new();
        w.start_response("CreateOpenIDConnectProvider");
        w.start_result("CreateOpenIDConnectProvider");
        w.write_element("OpenIDConnectProviderArn", &arn);
        w.end_element("CreateOpenIDConnectProviderResult");
        w.write_response_metadata(&request_id);
        w.end_element("CreateOpenIDConnectProviderResponse");

        Ok((w.into_string(), request_id))
    }

    /// Get an OIDC identity provider.
    pub fn get_open_id_connect_provider(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let arn = get_required_param(params, "OpenIDConnectProviderArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        let provider = self.store.oidc_providers.get(arn).ok_or_else(|| {
            IamError::no_such_entity(format!("OpenIDConnect provider not found for ARN: {arn}"))
        })?;

        let mut w = XmlWriter::new();
        w.start_response("GetOpenIDConnectProvider");
        w.start_result("GetOpenIDConnectProvider");
        w.write_element("Url", &provider.url);
        w.start_element("ClientIDList");
        for cid in &provider.client_id_list {
            w.write_element("member", cid);
        }
        w.end_element("ClientIDList");
        w.start_element("ThumbprintList");
        for tp in &provider.thumbprint_list {
            w.write_element("member", tp);
        }
        w.end_element("ThumbprintList");
        w.write_element("CreateDate", &provider.create_date);
        if !provider.tags.is_empty() {
            write_tags_xml(&mut w, &provider.tags);
        }
        w.end_element("GetOpenIDConnectProviderResult");
        w.write_response_metadata(&request_id);
        w.end_element("GetOpenIDConnectProviderResponse");

        Ok((w.into_string(), request_id))
    }

    /// Delete an OIDC identity provider.
    pub fn delete_open_id_connect_provider(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let arn = get_required_param(params, "OpenIDConnectProviderArn")?;
        let request_id = uuid::Uuid::new_v4().to_string();

        if self.store.oidc_providers.remove(arn).is_none() {
            return Err(IamError::no_such_entity(format!(
                "OpenIDConnect provider not found for ARN: {arn}"
            )));
        }

        debug!(arn, "deleted OIDC provider");
        Ok((
            empty_response("DeleteOpenIDConnectProvider", &request_id),
            request_id,
        ))
    }

    /// List all OIDC identity providers.
    pub fn list_open_id_connect_providers(
        &self,
        _params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut w = XmlWriter::new();
        w.start_response("ListOpenIDConnectProviders");
        w.start_result("ListOpenIDConnectProviders");
        w.start_element("OpenIDConnectProviderList");
        for entry in &self.store.oidc_providers {
            w.start_element("member");
            w.write_element("Arn", &entry.value().arn);
            w.end_element("member");
        }
        w.end_element("OpenIDConnectProviderList");
        w.end_element("ListOpenIDConnectProvidersResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListOpenIDConnectProvidersResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Policy Tags ----

    /// Tag a managed policy.
    pub fn tag_policy(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let new_tags = parse_tag_list(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut policy = self.store.policies.get_mut(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        let mut merged_tags = policy.tags.clone();
        for (key, value) in &new_tags {
            if let Some(existing) = merged_tags.iter_mut().find(|(k, _)| k == key) {
                existing.1.clone_from(value);
            } else {
                merged_tags.push((key.clone(), value.clone()));
            }
        }

        if merged_tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        policy.tags = merged_tags;

        debug!(policy_arn, count = new_tags.len(), "tagged policy");
        Ok((empty_response("TagPolicy", &request_id), request_id))
    }

    /// Remove tags from a managed policy.
    pub fn untag_policy(&self, params: &[(String, String)]) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let tag_keys = parse_string_list(params, "TagKeys");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut policy = self.store.policies.get_mut(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        policy.tags.retain(|(k, _)| !tag_keys.contains(k));

        debug!(policy_arn, count = tag_keys.len(), "untagged policy");
        Ok((empty_response("UntagPolicy", &request_id), request_id))
    }

    /// List tags for a managed policy.
    pub fn list_policy_tags(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let policy_arn = get_required_param(params, "PolicyArn")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let policy = self.store.policies.get(policy_arn).ok_or_else(|| {
            IamError::no_such_entity(format!("Policy {policy_arn} does not exist."))
        })?;

        let (page, is_truncated, next_marker) = paginate(&policy.tags, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListPolicyTags");
        w.start_result("ListPolicyTags");
        write_tags_xml(&mut w, page);
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListPolicyTagsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListPolicyTagsResponse");

        Ok((w.into_string(), request_id))
    }

    // ---- Instance Profile Tags ----

    /// Tag an instance profile.
    pub fn tag_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let ip_name = get_required_param(params, "InstanceProfileName")?;
        let new_tags = parse_tag_list(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut ip = self
            .store
            .instance_profiles
            .get_mut(ip_name)
            .ok_or_else(|| {
                IamError::no_such_entity(format!("Instance profile {ip_name} does not exist."))
            })?;

        let mut merged_tags = ip.tags.clone();
        for (key, value) in &new_tags {
            if let Some(existing) = merged_tags.iter_mut().find(|(k, _)| k == key) {
                existing.1.clone_from(value);
            } else {
                merged_tags.push((key.clone(), value.clone()));
            }
        }

        if merged_tags.len() > MAX_TAGS_PER_ENTITY {
            return Err(IamError::limit_exceeded(format!(
                "Cannot exceed {MAX_TAGS_PER_ENTITY} tags per entity"
            )));
        }

        ip.tags = merged_tags;

        debug!(ip_name, count = new_tags.len(), "tagged instance profile");
        Ok((
            empty_response("TagInstanceProfile", &request_id),
            request_id,
        ))
    }

    /// Remove tags from an instance profile.
    pub fn untag_instance_profile(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let ip_name = get_required_param(params, "InstanceProfileName")?;
        let tag_keys = parse_string_list(params, "TagKeys");
        let request_id = uuid::Uuid::new_v4().to_string();

        let mut ip = self
            .store
            .instance_profiles
            .get_mut(ip_name)
            .ok_or_else(|| {
                IamError::no_such_entity(format!("Instance profile {ip_name} does not exist."))
            })?;

        ip.tags.retain(|(k, _)| !tag_keys.contains(k));

        debug!(ip_name, count = tag_keys.len(), "untagged instance profile");
        Ok((
            empty_response("UntagInstanceProfile", &request_id),
            request_id,
        ))
    }

    /// List tags for an instance profile.
    pub fn list_instance_profile_tags(
        &self,
        params: &[(String, String)],
    ) -> Result<(String, String), IamError> {
        let ip_name = get_required_param(params, "InstanceProfileName")?;
        let (marker, max_items) = parse_pagination(params);
        let request_id = uuid::Uuid::new_v4().to_string();

        let ip = self.store.instance_profiles.get(ip_name).ok_or_else(|| {
            IamError::no_such_entity(format!("Instance profile {ip_name} does not exist."))
        })?;

        let (page, is_truncated, next_marker) = paginate(&ip.tags, marker, max_items);

        let mut w = XmlWriter::new();
        w.start_response("ListInstanceProfileTags");
        w.start_result("ListInstanceProfileTags");
        write_tags_xml(&mut w, page);
        w.write_bool_element("IsTruncated", is_truncated);
        if let Some(ref m) = next_marker {
            w.write_element("Marker", m);
        }
        w.end_element("ListInstanceProfileTagsResult");
        w.write_response_metadata(&request_id);
        w.end_element("ListInstanceProfileTagsResponse");

        Ok((w.into_string(), request_id))
    }
}
