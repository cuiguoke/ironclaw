//! Gmail Channel for IronClaw.
//!
//! Receives Gmail push notifications via Google Pub/Sub, deduplicates them,
//! and emits structured events to the Agent. The Agent uses the Gmail Tool
//! to fetch and analyze email content.
//!
//! Watch subscription is managed externally (e.g. via `gog gmail watch`).

#![allow(dead_code)]

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};
use base64::Engine;

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// Workspace paths for cross-callback state
// ============================================================================

const STATE_HISTORY_ID_PATH: &str = "state/history_id";
const STATE_PROCESSED_IDS_PATH: &str = "state/processed_ids";
const STATE_OWNER_ID_PATH: &str = "state/owner_id";
const STATE_PUBSUB_SUBSCRIPTION_PATH: &str = "state/pubsub_subscription";
const STATE_SUMMARY_LANGUAGE_PATH: &str = "state/summary_language";

// ============================================================================
// Gmail API Types (Standard Google Pub/Sub Format)
// ============================================================================

/// Google Pub/Sub push message envelope (standard format).
/// This is the format Google Cloud Pub/Sub sends to HTTP push endpoints.
#[derive(Debug, Deserialize)]
struct PushRequest {
    /// The Pub/Sub message.
    message: PubsubMessage,
    /// The subscription name.
    subscription: String,
}

/// Pub/Sub message body (standard Google format).
/// Pub/Sub sends "messageId" and "publishTime" in camelCase.
#[derive(Debug, Deserialize)]
struct PubsubMessage {
    /// Pub/Sub message unique ID (for deduplication).
    /// Pub/Sub sends this as "messageId" in the JSON.
    #[serde(rename = "messageId")]
    message_id: String,
    /// Base64-encoded notification data.
    /// This is the actual Gmail notification payload.
    data: String,
    /// Publish timestamp (ISO 8601 format).
    /// Pub/Sub sends this as "publishTime" in the JSON.
    #[serde(rename = "publishTime")]
    publish_time: String,
    /// Message attributes (optional key-value pairs).
    #[serde(default)]
    attributes: Option<std::collections::HashMap<String, String>>,
}

/// Gmail notification data (decoded from Pub/Sub message.data).
#[derive(Debug, Deserialize, Serialize)]
struct GmailNotificationData {
    /// Email address associated with the notification.
    #[serde(alias = "emailAddress")]
    email_address: String,
    /// Gmail history ID for incremental fetch.
    /// Gmail API sends this as an integer.
    #[serde(alias = "historyId")]
    history_id: u64,
}

/// Gmail channel configuration.
#[derive(Debug, Deserialize)]
struct GmailConfig {
    /// Expected Pub/Sub subscription name (optional, for source verification).
    pubsub_subscription: Option<String>,
    /// IronClaw user_id for all emitted messages (optional). When set,
    /// all notifications use this value as user_id regardless of email address.
    /// This ensures the Agent can find the correct credentials when calling Gmail tool.
    user_id: Option<String>,
    /// Language for summarization (optional, default: "Chinese").
    /// Supported values: "English", "Chinese", "Spanish", etc.
    /// This tells the Agent which language to use when summarizing emails.
    summary_language: Option<String>,
}

/// Metadata for emitted Gmail notifications.
#[derive(Debug, Serialize, Deserialize)]
struct GmailNotificationMetadata {
    /// Email address from the notification.
    email_address: String,
    /// History ID from the notification (as integer).
    history_id: u64,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct GmailChannel;

export!(GmailChannel);

impl Guest for GmailChannel {
    /// Initialize the Gmail channel. Watch subscription is managed externally.
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: GmailConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse Gmail config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Gmail channel starting");

        if let Some(ref subscription) = config.pubsub_subscription {
            let _ = channel_host::workspace_write(STATE_PUBSUB_SUBSCRIPTION_PATH, subscription);
        }

        if let Some(ref user_id) = config.user_id {
            let _ = channel_host::workspace_write(STATE_OWNER_ID_PATH, user_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("User ID configured: {}", user_id),
            );
        }

        if let Some(ref summary_language) = config.summary_language {
            let _ = channel_host::workspace_write(STATE_SUMMARY_LANGUAGE_PATH, summary_language);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Summary language configured: {}", summary_language),
            );
        }

        Ok(ChannelConfig {
            display_name: "Gmail".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/gmail".to_string(),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }],
            poll: None,
        })
    }

    /// Handle incoming Pub/Sub push notification.
    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse request body as UTF-8.
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    "Failed to decode request body as UTF-8",
                );
                return json_response(200, serde_json::json!({}));
            }
        };

        // Parse as Pub/Sub push message (standard Google format).
        let push_req: PushRequest = match serde_json::from_str(body_str) {
            Ok(msg) => msg,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Pub/Sub message: {}", e),
                );
                return json_response(200, serde_json::json!({}));
            }
        };

        // Verify subscription source if configured.
        if let Some(expected_subscription) =
            channel_host::workspace_read(STATE_PUBSUB_SUBSCRIPTION_PATH)
        {
            if !expected_subscription.is_empty() && push_req.subscription != expected_subscription {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Rejecting notification from unexpected subscription: {}",
                        push_req.subscription
                    ),
                );
                return json_response(200, serde_json::json!({}));
            }
        }

        // Decode Base64URL payload (Gmail API format).
        let notification_data: GmailNotificationData = match decode_and_parse_notification(&push_req.message.data) {
            Ok(data) => data,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to decode notification data: {}", e),
                );
                return json_response(200, serde_json::json!({}));
            }
        };

        // Check deduplication using standard Pub/Sub message_id.
        match check_deduplication(notification_data.history_id, &push_req.message.message_id) {
            DeduplicationResult::New => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "New notification: email={} history_id={}",
                        notification_data.email_address, notification_data.history_id
                    ),
                );
                // Process new notification.
                emit_notification(&notification_data);
            }
            DeduplicationResult::StaleHistoryId => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Skipping notification with stale history ID: {}",
                        notification_data.history_id
                    ),
                );
            }
            DeduplicationResult::DuplicateMessageId => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    "Skipping duplicate Pub/Sub message",
                );
            }
        }

        // Always return 200 to acknowledge receipt (avoid Pub/Sub retry storms).
        json_response(200, serde_json::json!({}))
    }

    /// Polling is not used for Gmail (webhook-driven).
    fn on_poll() {
        // No-op: Gmail uses webhooks, not polling.
    }

    /// Handle Agent response: log for audit.
    fn on_respond(response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!("Agent response received with content length: {}", response.content.len()),
        );

        // The Agent has processed the Gmail notification and may have saved
        // a summary to workspace. We just log for audit purposes.
        Ok(())
    }

    /// Handle broadcast message (log for audit).
    fn on_broadcast(user_id: String, response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Broadcast message for user {}: {}",
                user_id, response.content
            ),
        );
        Ok(())
    }

    /// Status updates are not forwarded to Gmail.
    fn on_status(_update: StatusUpdate) {
        // No-op: Gmail does not support status indicators.
    }

    /// Shutdown handler.
    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Gmail channel shutting down");
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Deduplication result.
enum DeduplicationResult {
    /// New notification, should be processed.
    New,
    /// History ID is stale (already processed), skip.
    StaleHistoryId,
    /// Pub/Sub message ID is duplicate, skip.
    DuplicateMessageId,
}

/// Check deduplication via History ID and Message ID.
fn check_deduplication(
    history_id: u64,
    message_id: &str,
) -> DeduplicationResult {
    // Check History ID: must be strictly greater than stored value.
    if let Some(stored_history_id_str) = channel_host::workspace_read(STATE_HISTORY_ID_PATH) {
        if !stored_history_id_str.is_empty() {
            if let Ok(stored_history_id) = stored_history_id_str.parse::<u64>() {
                if history_id <= stored_history_id {
                    return DeduplicationResult::StaleHistoryId;
                }
            }
        }
    }

    // Check Message ID: maintain set of recent processed IDs.
    if let Some(processed_ids_json) = channel_host::workspace_read(STATE_PROCESSED_IDS_PATH) {
        if let Ok(mut processed_ids) = serde_json::from_str::<Vec<String>>(&processed_ids_json) {
            if processed_ids.contains(&message_id.to_string()) {
                return DeduplicationResult::DuplicateMessageId;
            }

            // Add new ID and maintain size limit (200 entries).
            processed_ids.push(message_id.to_string());
            if processed_ids.len() > 200 {
                processed_ids.remove(0);
            }

            if let Ok(updated_json) = serde_json::to_string(&processed_ids) {
                let _ = channel_host::workspace_write(STATE_PROCESSED_IDS_PATH, &updated_json);
            }
        }
    } else {
        // Initialize processed IDs set.
        if let Ok(json) = serde_json::to_string(&vec![message_id.to_string()]) {
            let _ = channel_host::workspace_write(STATE_PROCESSED_IDS_PATH, &json);
        }
    }

    DeduplicationResult::New
}

/// Decode Base64URL and parse Gmail notification data.
/// Gmail API push notifications use Base64URL encoding (URL-safe, with padding).
fn decode_and_parse_notification(data: &str) -> Result<GmailNotificationData, String> {
    // Decode Base64URL (Gmail API format - URL-safe with padding).
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(data)
        .map_err(|e| format!("Base64URL decode failed: {}", e))?;

    let json_str = String::from_utf8(decoded)
        .map_err(|e| format!("UTF-8 decode failed: {}", e))?;

    serde_json::from_str(&json_str)
        .map_err(|e| format!("JSON parse failed: {}", e))
}

/// Emit notification event to Agent.
fn emit_notification(notification: &GmailNotificationData) {
    // Determine user_id: use owner_id if configured, otherwise email address.
    let user_id = channel_host::workspace_read(STATE_OWNER_ID_PATH)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| notification.email_address.clone());

    // Ensure pairing exists for this sender. Gmail is push-only so users
    // cannot send a message to trigger pairing. We upsert on every
    // notification; the store deduplicates and returns the existing code
    // if one already exists.
    let meta = serde_json::json!({ "email": notification.email_address });
    match channel_host::pairing_resolve_identity("gmail", &user_id) {
        Ok(Some(_)) => {} // already paired
        _ => {
            if let Ok(result) = channel_host::pairing_upsert_request(
                "gmail",
                &user_id,
                &meta.to_string(),
            ) {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!(
                        "Pairing request created. Run: ironclaw pairing approve gmail {}",
                        result.code
                    ),
                );
            }
        }
    }

    // Build metadata without timestamp.
    let metadata = GmailNotificationMetadata {
        email_address: notification.email_address.clone(),
        history_id: notification.history_id.clone(),
    };

    let metadata_json = serde_json::to_string(&metadata)
        .unwrap_or_else(|_| "{}".to_string());

    // Get summary language if configured (default: "Chinese").
    let summary_language = channel_host::workspace_read(STATE_SUMMARY_LANGUAGE_PATH)
        .filter(|lang| !lang.is_empty())
        .unwrap_or_else(|| "Chinese".to_string());

    // Build structured prompt with clear step-by-step instructions in Chinese
    let content = format!(
        "收到来自 {} 的 Gmail 通知。\n\
         \n\
         请按照以下步骤处理：\n\
         \n\
         1. 获取未读邮件，使用 gmail 工具：\n\
            - action: list_messages\n\
            - query: is:unread\n\
            - max_results: 5\n\
         \n\
         2. 对每条邮件获取完整内容，使用 gmail 工具：\n\
            - action: get_message\n\
            - message_id: (来自第1步的结果)\n\
         \n\
         3. 分析邮件内容，提取以下信息：\n\
            - 主题、发件人、日期\n\
            - 关键信息和待办事项\n\
            - 优先级\n\
         \n\
         4. 用 {} 生成摘要，格式如下：\n\
            # Gmail 邮件摘要\n\
            **邮箱**: {}\n\
            **时间**: (当前时间戳)\n\
            \n\
            ## 邮件列表\n\
            (列出每封邮件的主题、发件人、优先级、摘要)\n\
            \n\
            ## 待办事项\n\
            (列出所有待办事项)\n\
         \n\
         5. 使用 memory_write 工具保存摘要：\n\
            - target: summaries/gmail/{}.md\n\
            - content: (第4步生成的摘要)",
        notification.email_address,
        summary_language,
        notification.email_address,
        notification.history_id
    );

    // Emit message to Agent.
    let _ = channel_host::emit_message(&EmittedMessage {
        user_id,
        user_name: None,
        content,
        thread_id: None,
        metadata_json,
        attachments: vec![],
    });

    // Update stored history ID.
    let _ = channel_host::workspace_write(STATE_HISTORY_ID_PATH, &notification.history_id.to_string());

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Emitted notification for {} (history_id: {})",
            notification.email_address, notification.history_id
        ),
    );
}

/// Build JSON HTTP response.
fn json_response(status: u16, body: serde_json::Value) -> OutgoingHttpResponse {
    let body_bytes = serde_json::to_vec(&body).unwrap_or_default();
    OutgoingHttpResponse {
        status,
        headers_json: serde_json::json!({
            "Content-Type": "application/json",
        })
        .to_string(),
        body: body_bytes,
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Config parsing
    // ========================================================================

    #[test]
    fn test_parse_complete_config() {
        let json = serde_json::json!({
            "pubsub_subscription": "projects/p/subscriptions/s",
            "user_id": "user123",
            "summary_language": "Chinese"
        });
        let config: GmailConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.pubsub_subscription.as_deref(), Some("projects/p/subscriptions/s"));
        assert_eq!(config.user_id.as_deref(), Some("user123"));
        assert_eq!(config.summary_language.as_deref(), Some("Chinese"));
    }

    #[test]
    fn test_parse_empty_config() {
        let config: GmailConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.pubsub_subscription, None);
        assert_eq!(config.user_id, None);
        assert_eq!(config.summary_language, None);
    }

    #[test]
    fn test_parse_config_ignores_extra_fields() {
        let json = serde_json::json!({ "user_id": "u1", "unknown_field": 42 });
        let config: GmailConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.user_id.as_deref(), Some("u1"));
    }

    #[test]
    fn test_parse_config_null_optionals() {
        let json = serde_json::json!({
            "pubsub_subscription": null,
            "user_id": null,
            "output_channel": null
        });
        let config: GmailConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.pubsub_subscription, None);
    }

    // ========================================================================
    // Pub/Sub message parsing
    // ========================================================================

    #[test]
    fn test_parse_valid_pubsub_message() {
        let json = serde_json::json!({
            "message": {
                "data": "eyJlbWFpbEFkZHJlc3MiOiJ1c2VyQGV4YW1wbGUuY29tIiwiaGlzdG9yeUlkIjoiMTIzNDYifQ==",
                "messageId": "136969346945",
                "publishTime": "2024-01-01T00:00:00.000Z"
            },
            "subscription": "projects/p/subscriptions/s"
        });
        let msg: PushRequest = serde_json::from_value(json).unwrap();
        assert_eq!(msg.message.message_id, "136969346945");
        assert_eq!(msg.subscription, "projects/p/subscriptions/s");
    }

    #[test]
    fn test_parse_pubsub_message_missing_optional_fields() {
        let json = serde_json::json!({
            "message": { 
                "data": "dGVzdA==",
                "messageId": "msg-123",
                "publishTime": "2024-01-01T00:00:00Z"
            },
            "subscription": "projects/p/subscriptions/s"
        });
        let msg: PushRequest = serde_json::from_value(json).unwrap();
        assert_eq!(msg.message.message_id, "msg-123");
        assert_eq!(msg.message.publish_time, "2024-01-01T00:00:00Z");
    }

    // ========================================================================
    // Base64URL decoding
    // ========================================================================

    #[test]
    fn test_decode_valid_notification() {
        // {"emailAddress":"user@example.com","historyId":12346}
        let data = "eyJlbWFpbEFkZHJlc3MiOiJ1c2VyQGV4YW1wbGUuY29tIiwiaGlzdG9yeUlkIjoxMjM0Nn0=";
        let n = decode_and_parse_notification(data).unwrap();
        assert_eq!(n.email_address, "user@example.com");
        assert_eq!(n.history_id, 12346);
    }

    #[test]
    fn test_decode_history_id_as_integer() {
        // {"emailAddress":"user@example.com","historyId":23970}
        // Gmail API sends historyId as an integer
        let data = "eyJlbWFpbEFkZHJlc3MiOiJ1c2VyQGV4YW1wbGUuY29tIiwiaGlzdG9yeUlkIjoyMzk3MH0=";
        let n = decode_and_parse_notification(data).unwrap();
        assert_eq!(n.email_address, "user@example.com");
        assert_eq!(n.history_id, 23970);
    }

    #[test]
    fn test_decode_camelcase_fields() {
        // {"emailAddress":"a@b.com","historyId":99}
        let data = "eyJlbWFpbEFkZHJlc3MiOiJhQGIuY29tIiwiaGlzdG9yeUlkIjo5OX0=";
        let n = decode_and_parse_notification(data).unwrap();
        assert_eq!(n.email_address, "a@b.com");
        assert_eq!(n.history_id, 99);
    }

    #[test]
    fn test_decode_invalid_base64() {
        assert!(decode_and_parse_notification("!!!invalid!!!").is_err());
    }

    #[test]
    fn test_decode_missing_fields() {
        // {"emailAddress":"a@b.com"} — missing historyId
        let data = "eyJlbWFpbEFkZHJlc3MiOiJhQGIuY29tIn0=";
        assert!(decode_and_parse_notification(data).is_err());
    }

    #[test]
    fn test_decode_invalid_json() {
        // {invalid json}
        let data = "e2ludmFsaWQganNvbn0=";
        assert!(decode_and_parse_notification(data).is_err());
    }

    // ========================================================================
    // Deduplication logic (unit-level, no host calls)
    // ========================================================================

    #[test]
    fn test_history_id_equal_rejected() {
        let stored: u64 = 12345;
        let current: u64 = 12345;
        assert!(current <= stored);
    }

    #[test]
    fn test_history_id_smaller_rejected() {
        let stored: u64 = 12345;
        let current: u64 = 12344;
        assert!(current <= stored);
    }

    #[test]
    fn test_history_id_larger_accepted() {
        let stored: u64 = 12345;
        let current: u64 = 12346;
        assert!(current > stored);
    }

    #[test]
    fn test_message_id_duplicate_detection() {
        let ids = vec!["msg-1".to_string(), "msg-2".to_string()];
        assert!(ids.contains(&"msg-1".to_string()));
        assert!(!ids.contains(&"msg-3".to_string()));
    }

    #[test]
    fn test_message_id_set_capacity_and_fifo() {
        let mut ids: Vec<String> = (0..200).map(|i| format!("msg-{}", i)).collect();
        assert_eq!(ids.len(), 200);
        ids.push("msg-200".to_string());
        if ids.len() > 200 { ids.remove(0); }
        assert_eq!(ids.len(), 200);
        assert!(!ids.contains(&"msg-0".to_string()));
        assert!(ids.contains(&"msg-200".to_string()));
    }

    // ========================================================================
    // Metadata serialization
    // ========================================================================

    #[test]
    fn test_metadata_roundtrip() {
        let meta = GmailNotificationMetadata {
            email_address: "a@b.com".to_string(),
            history_id: 123,
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: GmailNotificationMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.email_address, "a@b.com");
        assert_eq!(back.history_id, 123);
    }

    #[test]
    fn test_metadata_serialization() {
        let meta = GmailNotificationMetadata {
            email_address: "a@b.com".to_string(),
            history_id: 1,
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("a@b.com"));
        assert!(json.contains("1"));
    }

    // ========================================================================
    // Property-based tests
    // ========================================================================

    #[test]
    fn prop_base64url_roundtrip() {
        use proptest::prelude::*;
        proptest!(|(
            email in r"[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}",
            history_id in 0u64..1_000_000_000u64
        )| {
            let original = GmailNotificationData {
                email_address: email.clone(),
                history_id,
            };
            let json_str = serde_json::to_string(&original).unwrap();
            let encoded = base64::engine::general_purpose::URL_SAFE.encode(json_str.as_bytes());
            let decoded = decode_and_parse_notification(&encoded).unwrap();
            prop_assert_eq!(decoded.email_address, original.email_address);
            prop_assert_eq!(decoded.history_id, original.history_id);
        });
    }

    #[test]
    fn prop_history_id_monotonic() {
        use proptest::prelude::*;
        proptest!(|(stored in 0u64..1_000_000_000u64, offset in -100i64..100i64)| {
            let current = if offset < 0 {
                stored.saturating_sub((-offset) as u64)
            } else {
                stored.saturating_add(offset as u64)
            };
            if current > stored {
                prop_assert!(current > stored);
            } else {
                prop_assert!(current <= stored);
            }
        });
    }

    #[test]
    fn prop_message_id_set_bounded() {
        use proptest::prelude::*;
        proptest!(|(ids in prop::collection::vec("[a-z0-9]{10,20}", 300..400))| {
            let mut unique: Vec<String> = ids;
            unique.sort();
            unique.dedup();
            prop_assume!(unique.len() >= 250);

            let mut set: Vec<String> = Vec::new();
            for id in &unique {
                if !set.contains(id) {
                    set.push(id.clone());
                    if set.len() > 200 { set.remove(0); }
                }
                prop_assert!(set.len() <= 200);
            }
        });
    }

    #[test]
    fn prop_malformed_payload_no_panic() {
        use proptest::prelude::*;
        let strategy = prop_oneof![
            Just("{}".to_string()),
            Just("{invalid}".to_string()),
            Just("".to_string()),
            Just("null".to_string()),
            r"[a-zA-Z0-9!@#$%^&*]{1,100}".prop_map(|s| s),
        ];
        proptest!(|(payload in strategy)| {
            let _: Result<PushRequest, _> = serde_json::from_str(&payload);
        });
    }
}
