//! Direct HTTP provider for OpenAI-compatible APIs that only support a basic
//! subset of the spec (string content, no content arrays, limited JSON Schema).
//!
//! Used for providers like 智谱 AI GLM domestic API (open.bigmodel.cn) that
//! implement an early/simplified version of the OpenAI Chat Completions API.
//! Bypasses rig-core serialization entirely to avoid incompatibilities.

use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value as JsonValue, json};

use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
    Role, ToolCall as IronToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition as IronToolDefinition, strip_unsupported_completion_params,
    strip_unsupported_tool_params,
};

/// OpenAI-compatible provider that sends plain string content and sanitized
/// tool schemas, bypassing rig-core serialization entirely.
///
/// Handles the following GLM incompatibilities:
/// - content must be a string, not an array
/// - tool schemas must not contain anyOf/oneOf/allOf
/// - type field must be a string, not an array like ["string", "null"]
/// - additionalProperties is not supported
pub struct BasicOpenAiProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: SecretString,
    model_name: String,
    input_cost: Decimal,
    output_cost: Decimal,
    unsupported_params: HashSet<String>,
}

impl BasicOpenAiProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
        unsupported_params: Vec<String>,
        extra_headers: Vec<(String, String)>,
        request_timeout: Duration,
    ) -> Result<Self, LlmError> {
        let model = model_name.into();
        let (input_cost, output_cost) =
            costs::model_cost(&model).unwrap_or_else(costs::default_cost);

        let mut headers = reqwest::header::HeaderMap::new();
        for (k, v) in &extra_headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                headers.insert(name, val);
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(request_timeout)
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: model.clone(),
                reason: format!("Failed to build HTTP client: {e}"),
            })?;

        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: SecretString::from(api_key.into()),
            model_name: model,
            input_cost,
            output_cost,
            unsupported_params: unsupported_params.into_iter().collect(),
        })
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Convert IronClaw messages to plain OpenAI message objects.
    /// Content is always a string — never an array.
    fn build_messages(&self, messages: &[ChatMessage]) -> Vec<JsonValue> {
        let mut result: Vec<JsonValue> = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // Merge consecutive system messages into one
                    if let Some(last) = result.last_mut() {
                        if last.get("role").and_then(|r| r.as_str()) == Some("system") {
                            let prev = last["content"].as_str().unwrap_or("").to_string();
                            last["content"] = json!(format!("{}\n{}", prev, msg.content));
                            continue;
                        }
                    }
                    result.push(json!({ "role": "system", "content": msg.content }));
                }
                Role::User => {
                    result.push(json!({ "role": "user", "content": msg.content }));
                }
                Role::Assistant => {
                    if let Some(ref tool_calls) = msg.tool_calls {
                        let tc_json: Vec<JsonValue> = tool_calls
                            .iter()
                            .map(|tc| json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string()
                                }
                            }))
                            .collect();
                        let mut obj = json!({ "role": "assistant", "tool_calls": tc_json });
                        // GLM requires content to not be null when tool_calls present
                        obj["content"] = json!(if msg.content.is_empty() { "" } else { &msg.content });
                        result.push(obj);
                    } else {
                        result.push(json!({ "role": "assistant", "content": msg.content }));
                    }
                }
                Role::Tool => {
                    let tool_call_id = msg
                        .tool_call_id
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string());
                    result.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_call_id,
                        "content": msg.content
                    }));
                }
            }
        }

        result
    }

    /// Build sanitized tool definitions for GLM.
    fn build_tools(&self, tools: &[IronToolDefinition]) -> Vec<JsonValue> {
        tools
            .iter()
            .map(|t| {
                let params = sanitize_schema(t.parameters.clone());
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": params
                    }
                })
            })
            .collect()
    }

    async fn post(&self, body: JsonValue) -> Result<JsonValue, LlmError> {
        let resp = self
            .client
            .post(&self.chat_completions_url())
            .bearer_auth(self.api_key.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: e.to_string(),
            })?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| LlmError::RequestFailed {
            provider: self.model_name.clone(),
            reason: e.to_string(),
        })?;

        if !status.is_success() {
            return Err(LlmError::RequestFailed {
                provider: self.model_name.clone(),
                reason: format!(
                    "HttpError: Invalid status code {} with message: {}",
                    status, text
                ),
            });
        }

        serde_json::from_str(&text).map_err(|e| LlmError::InvalidResponse {
            provider: self.model_name.clone(),
            reason: format!("Failed to parse response JSON: {e}: {text}"),
        })
    }

    fn parse_finish_reason(choice: &JsonValue) -> FinishReason {
        match choice
            .get("finish_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
        {
            "tool_calls" => FinishReason::ToolUse,
            "length" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        }
    }

    fn parse_usage(resp: &JsonValue) -> (u32, u32) {
        let u = resp.get("usage");
        let input = u
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        let output = u
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as u32;
        (input, output)
    }

    fn parse_tool_calls(choice: &JsonValue) -> Vec<IronToolCall> {
        let Some(tcs) = choice
            .get("message")
            .and_then(|m| m.get("tool_calls"))
            .and_then(|t| t.as_array())
        else {
            return Vec::new();
        };

        tcs.iter()
            .filter_map(|tc| {
                let id = tc.get("id")?.as_str()?.to_string();
                let func = tc.get("function")?;
                let name = func.get("name")?.as_str()?.to_string();
                let args_str = func.get("arguments")?.as_str().unwrap_or("{}");
                let arguments: JsonValue =
                    serde_json::from_str(args_str).unwrap_or(json!({}));
                Some(IronToolCall { id, name, arguments })
            })
            .collect()
    }
}

#[async_trait]
impl LlmProvider for BasicOpenAiProvider {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (self.input_cost, self.output_cost)
    }

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        strip_unsupported_completion_params(&self.unsupported_params, &mut request);

        let messages = self.build_messages(&request.messages);
        let mut body = json!({
            "model": self.model_name,
            "messages": messages,
        });
        if let Some(t) = request.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = request.max_tokens {
            body["max_tokens"] = json!(m);
        }

        let resp = self.post(body).await?;
        let choice = &resp["choices"][0];
        let content = choice["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let (input_tokens, output_tokens) = Self::parse_usage(&resp);

        Ok(CompletionResponse {
            content,
            input_tokens,
            output_tokens,
            finish_reason: Self::parse_finish_reason(choice),
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        mut request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        strip_unsupported_tool_params(&self.unsupported_params, &mut request);
        crate::llm::provider::sanitize_tool_messages(&mut request.messages);

        let messages = self.build_messages(&request.messages);
        let tools = self.build_tools(&request.tools);

        let mut body = json!({
            "model": self.model_name,
            "messages": messages,
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
            if let Some(ref choice) = request.tool_choice {
                body["tool_choice"] = json!(choice);
            }
        }
        if let Some(t) = request.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = request.max_tokens {
            body["max_tokens"] = json!(m);
        }

        tracing::trace!(
            model = %self.model_name,
            num_messages = request.messages.len(),
            num_tools = request.tools.len(),
            temperature = ?request.temperature,
            max_tokens = ?request.max_tokens,
            tool_choice = ?request.tool_choice,
            "LLM request details (basic provider)"
        );

        let resp = self.post(body).await?;
        let choice = &resp["choices"][0];
        let content = choice["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
        let tool_calls = Self::parse_tool_calls(choice);
        let (input_tokens, output_tokens) = Self::parse_usage(&resp);

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason: Self::parse_finish_reason(choice),
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

/// Sanitize a JSON Schema for basic OpenAI-compatible providers.
///
/// Removes: anyOf, oneOf, allOf, additionalProperties, examples.
/// Normalizes: array type → first non-null scalar.
/// Adds: default "string" type for properties missing a type field.
fn sanitize_schema(mut schema: JsonValue) -> JsonValue {
    sanitize_recursive(&mut schema);
    schema
}

fn sanitize_recursive(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    obj.remove("anyOf");
    obj.remove("oneOf");
    obj.remove("allOf");
    obj.remove("additionalProperties");
    obj.remove("examples");

    // ["string", "null"] → "string"
    if let Some(type_val) = obj.get("type").cloned() {
        if let JsonValue::Array(arr) = type_val {
            let first = arr
                .iter()
                .find(|v| v.as_str() != Some("null"))
                .cloned()
                .unwrap_or(json!("string"));
            obj.insert("type".to_string(), first);
        }
    }

    // Recurse into properties, adding default type where missing
    if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
        for prop in props.values_mut() {
            sanitize_recursive(prop);
            if let Some(prop_obj) = prop.as_object_mut() {
                if !prop_obj.contains_key("type")
                    && !prop_obj.contains_key("$ref")
                    && !prop_obj.contains_key("enum")
                {
                    prop_obj.insert("type".to_string(), json!("string"));
                }
            }
        }
    }

    // Recurse into array items
    if let Some(items) = obj.get_mut("items") {
        sanitize_recursive(items);
    }
}
