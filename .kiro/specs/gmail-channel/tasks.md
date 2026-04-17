# 实现计划: Gmail Channel

## 概述

基于 IronClaw 的 `sandboxed-channel` WIT 接口，实现一个 Rust WASM channel 插件，通过 Google Pub/Sub 推送通知被动接收 Gmail 邮件变更事件。项目结构参考 `channels-src/feishu/`，包含 `Cargo.toml`、`build.sh`、`gmail-channel.capabilities.json` 和 `src/lib.rs`。

## Tasks

- [x] 1. 搭建项目结构与基础框架
  - [x] 1.1 创建 `channels-src/gmail/Cargo.toml`
    - 参考 `channels-src/feishu/Cargo.toml`，创建 `gmail-channel` crate
    - `crate-type = ["cdylib"]`，依赖 `wit-bindgen`、`serde`、`serde_json`
    - 新增 `base64` 依赖（用于 Base64URL 解码 Pub/Sub 载荷）
    - 配置 `[profile.release]` 优化 WASM 体积（`opt-level = "s"`, `lto = true`, `strip = true`）
    - 设置 `[workspace]` 使其独立于父工作区
    - _需求: 8.1_

  - [x] 1.2 创建 `channels-src/gmail/build.sh`
    - 参考 `channels-src/feishu/build.sh`，编写 Gmail channel 的构建脚本
    - 使用 `cargo build --release --target wasm32-wasip2` 构建
    - 使用 `wasm-tools component new` 和 `wasm-tools strip` 处理输出
    - 输出文件为 `gmail-channel.wasm`
    - _需求: 8.1_

  - [x] 1.3 创建 `channels-src/gmail/gmail-channel.capabilities.json`
    - 定义 `type: "channel"`、`name: "gmail"`
    - 配置 Google OAuth 2.0 认证（`gmail.readonly` 权限范围）
    - 定义 HTTP 白名单：仅允许 `www.googleapis.com` 的 `/gmail/v1/users/me/watch` POST 请求
    - 配置 webhook 端点 `/webhook/gmail`，`allow_polling: false`
    - 配置凭证注入 `google_oauth_token`，bearer token 方式
    - 定义 setup 部分：`google_oauth_client_id` 和 `google_oauth_client_secret`
    - 设置 `workspace_prefix: "channels/gmail/"`
    - 定义 `config` 部分：`pubsub_topic`、`pubsub_subscription`、`label_ids`、`owner_id`、`output_channel`
    - _需求: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7_

  - [x] 1.4 创建 `channels-src/gmail/src/lib.rs` 基础骨架
    - 添加 `wit_bindgen::generate!` 宏调用，绑定 `sandboxed-channel` world
    - 导入 `exports::near::agent::channel::Guest` 等生成类型
    - 定义 `GmailChannel` 结构体并 `export!`
    - 实现 `Guest` trait 的所有方法（`on_start`、`on_http_request`、`on_poll`、`on_respond`、`on_broadcast`、`on_status`、`on_shutdown`），初始使用占位实现
    - _需求: 1.2_

- [x] 2. 实现配置解析与初始化 (`on_start`)
  - [x] 2.1 实现配置数据结构与解析逻辑
    - 定义 `GmailConfig` 结构体（`pubsub_topic`、`pubsub_subscription`、`label_ids`、`owner_id`、`output_channel`）
    - 实现 `default_label_ids()` 返回 `["INBOX"]`
    - 在 `on_start` 中解析 `config_json` 为 `GmailConfig`
    - 将所有配置项持久化到工作区存储（`state/pubsub_topic`、`state/owner_id`、`state/output_channel` 等）
    - _需求: 1.1, 1.5_

  - [x] 2.2 实现 Watch API 调用与订阅建立
    - 实现 `call_watch_api` 函数，调用 `POST https://www.googleapis.com/gmail/v1/users/me/watch`
    - 请求体包含 `topicName` 和 `labelIds`
    - 使用 `channel_host::http_request` 发送请求，附带 OAuth bearer token
    - 解析 `WatchResponse`（`historyId`、`expiration`）
    - 将初始 `historyId` 和 `watch_expiry` 持久化到工作区
    - Watch 失败时记录警告日志并继续启动（非致命错误）
    - _需求: 1.3, 1.4, 1.5_

  - [x] 2.3 完成 `on_start` 返回 `ChannelConfig`
    - 返回 `display_name: "Gmail"`
    - 注册 HTTP 端点 `/webhook/gmail`，方法 `POST`
    - 设置 `poll: None`（不使用轮询）
    - _需求: 1.2_

  - [x] 2.4 编写属性测试：配置解析完整性
    - **Property 13: 配置解析完整性**
    - 生成包含各种可选字段组合的有效配置 JSON，验证 `on_start` 正确提取所有字段并使用默认值填充缺失字段
    - **验证: 需求 1.1**

- [x] 3. 实现 Pub/Sub 推送通知接收与解析 (`on_http_request`)
  - [x] 3.1 定义 Pub/Sub 消息数据结构
    - 定义 `PubSubPushMessage`（`message`、`subscription`）
    - 定义 `PubSubMessage`（`data`、`message_id`、`publish_time`）
    - 定义 `GmailNotificationData`（`email_address`、`history_id`）
    - _需求: 2.1, 2.2_

  - [x] 3.2 实现 `on_http_request` 主流程
    - 解析请求体为 `PubSubPushMessage`
    - 验证 `subscription` 字段与预期订阅名称匹配（如已配置）
    - Base64URL 解码 `message.data` 字段（使用 `base64::engine::general_purpose::URL_SAFE_NO_PAD`）
    - 反序列化为 `GmailNotificationData`
    - 任何解析错误均返回 HTTP 200（避免 Pub/Sub 重试风暴）
    - _需求: 2.1, 2.2, 2.4, 2.5, 6.1_

  - [x] 3.3 编写属性测试：Base64URL 往返一致性
    - **Property 1: Pub/Sub 载荷 Base64URL 往返一致性**
    - 生成随机 email 地址和 history_id，验证 JSON → Base64URL 编码 → 解码 → JSON 反序列化的往返一致性
    - **验证: 需求 2.2**

  - [x] 3.4 编写属性测试：格式错误载荷返回 HTTP 200
    - **Property 3: 格式错误的载荷始终返回 HTTP 200**
    - 生成随机格式错误的请求体，验证 `on_http_request` 始终返回 HTTP 200
    - **验证: 需求 2.4**

  - [x] 3.5 编写属性测试：订阅来源验证
    - **Property 11: 订阅来源验证**
    - 生成随机订阅名称对，验证仅匹配时才处理通知
    - **验证: 需求 6.1**

- [x] 4. 实现去重引擎
  - [x] 4.1 实现 History ID 去重与 Message ID 去重
    - 实现 `check_deduplication` 函数，返回 `DeduplicationResult` 枚举
    - History ID 比较：通知的 `historyId` 必须严格大于已存储值
    - Message ID 集合：维护最近 200 条已处理的 Pub/Sub `messageId`
    - 集合满时淘汰最旧条目（FIFO）
    - 从工作区读写 `state/history_id` 和 `state/processed_ids`
    - _需求: 7.1, 7.2, 7.3, 7.4, 7.5_

  - [x] 4.2 将去重逻辑集成到 `on_http_request`
    - 在解析通知数据后、emit 之前调用去重检查
    - 重复通知返回 HTTP 200 并记录 debug 日志
    - _需求: 2.3_

  - [x] 4.3 编写属性测试：History ID 单调递增去重
    - **Property 2: History ID 单调递增去重**
    - 生成随机 history_id 对，验证仅当通知值严格大于已存储值时才接受
    - **验证: 需求 2.3, 7.1, 7.2**

  - [x] 4.4 编写属性测试：Message ID 去重集合有界性
    - **Property 12: Message ID 去重集合有界性**
    - 生成长随机 ID 序列，验证集合大小始终不超过 200 条
    - **验证: 需求 7.3, 7.4, 7.5**

- [x] 5. 检查点 - 确保所有测试通过
  - 确保所有测试通过，如有问题请向用户确认。

- [x] 6. 实现通知事件 emit 与元数据构建
  - [x] 6.1 实现 `NotificationEmitter` 逻辑
    - 定义 `GmailNotificationMetadata` 结构体（`email_address`、`history_id`、`notification_timestamp`、`output_channel`）
    - 构建结构化 prompt 内容，包含 email 地址、history_id 和使用 Gmail Tool 的指令
    - 确定 `user_id`：配置了 `owner_id` 时使用 `owner_id`，否则使用通知中的 email 地址
    - 调用 `channel_host::emit_message` 发送消息
    - emit 成功后更新工作区中的 `state/history_id`
    - _需求: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

  - [x] 6.2 编写属性测试：emit 包含结构化 prompt
    - **Property 4: 有效通知 emit 包含结构化 prompt**
    - 生成随机 email 和 history_id，验证 emit 消息包含所有必需信息
    - **验证: 需求 3.1, 3.2**

  - [x] 6.3 编写属性测试：元数据包含所有必需字段
    - **Property 5: Emit 元数据包含所有必需字段**
    - 生成随机通知和 output_channel 配置，验证元数据 JSON 字段完整性
    - **验证: 需求 3.3, 4.2**

  - [x] 6.4 编写属性测试：User ID 选择逻辑
    - **Property 6: User ID 选择逻辑**
    - 生成随机 owner_id 和 email 组合，验证 user_id 选择正确
    - **验证: 需求 3.4, 3.5**

  - [x] 6.5 编写属性测试：成功 emit 后 History ID 更新
    - **Property 7: 成功 emit 后 History ID 更新**
    - 验证处理完成后工作区中的 history_id 等于通知中的值
    - **验证: 需求 3.6**

- [x] 7. 实现响应处理与结果输出 (`on_respond` / `on_broadcast`)
  - [x] 7.1 实现 `on_respond` 响应路由
    - 解析 `response.metadata_json` 为 `GmailNotificationMetadata`
    - 当配置了 `output_channel` 时，记录审计日志（实际转发由 host/Agent 路由层处理）
    - 当未配置 `output_channel` 时，调用 `save_summary_to_workspace` 保存本地摘要
    - _需求: 4.1, 4.2, 4.3, 4.6_

  - [x] 7.2 实现本地摘要文件保存
    - 实现 `save_summary_to_workspace` 函数
    - 文件路径：`summaries/{timestamp}_{history_id}.md`
    - 文件内容包含 Markdown 格式的通知元数据（email、history_id）和 Agent 分析结果
    - 使用 `channel_host::workspace_write` 写入
    - _需求: 4.3, 4.4_

  - [x] 7.3 实现 `on_broadcast` 广播日志
    - 记录广播消息的审计日志
    - _需求: 4.5_

  - [x] 7.4 编写属性测试：本地摘要文件包含完整信息
    - **Property 8: 本地摘要文件包含完整信息**
    - 生成随机响应和元数据，验证摘要文件同时包含元数据和分析结果
    - **验证: 需求 4.3, 4.4**

- [x] 8. 实现订阅续期机制
  - [x] 8.1 实现 `check_and_renew_subscription` 函数
    - 从工作区读取 `state/watch_expiry`
    - 当过期时间与当前时间之差小于 24 小时时，调用 `call_watch_api` 续期
    - 续期成功后更新 `state/watch_expiry` 和 `state/history_id`
    - 续期失败时记录错误日志，不阻塞当前请求处理
    - _需求: 5.1, 5.2, 5.3, 5.4_

  - [x] 8.2 将续期检查集成到 `on_http_request`
    - 在每次成功处理 Pub/Sub 通知后，调用 `check_and_renew_subscription`
    - _需求: 5.1_

  - [x] 8.3 编写属性测试：订阅续期时间窗口
    - **Property 9: 订阅续期时间窗口**
    - 生成随机过期时间戳和当前时间，验证续期决策逻辑
    - **验证: 需求 5.1, 5.2**

- [x] 9. 实现辅助函数与收尾
  - [x] 9.1 实现 `json_response` 辅助函数
    - 参考 feishu channel 的 `json_response`，构建 JSON HTTP 响应
    - _需求: 2.4, 2.5_

  - [x] 9.2 实现 `on_poll`、`on_status`、`on_shutdown`
    - `on_poll`：空实现（不使用轮询）
    - `on_status`：空实现（Gmail 不支持状态指示器）
    - `on_shutdown`：记录关闭日志
    - _需求: 1.2_

  - [x] 9.3 定义工作区存储路径常量
    - 定义所有工作区路径常量（`STATE_HISTORY_ID_PATH`、`STATE_WATCH_EXPIRY_PATH`、`STATE_PROCESSED_IDS_PATH` 等）
    - 确保与设计文档中的路径表一致
    - _需求: 1.5, 7.1_

- [x] 10. 编写单元测试与集成测试
  - [x] 10.1 编写配置解析单元测试
    - 测试完整配置、最小配置（仅 `pubsub_topic`）、无效配置的解析
    - 验证 `label_ids` 默认值为 `["INBOX"]`
    - _需求: 1.1_

  - [x] 10.2 编写 Pub/Sub 消息解析单元测试
    - 测试有效消息解析、缺少字段、无效 Base64URL 数据
    - 测试 `GmailNotificationData` 反序列化
    - _需求: 2.1, 2.2_

  - [x] 10.3 编写去重逻辑单元测试
    - 测试 History ID 比较的边界情况（相等、小于、大于）
    - 测试 Message ID 集合的增删和容量限制
    - _需求: 7.1, 7.2, 7.3, 7.4, 7.5_

  - [x] 10.4 编写摘要文件格式化单元测试
    - 验证生成的 Markdown 文件包含所有必需字段
    - _需求: 4.3, 4.4_

  - [x] 10.5 编写 capabilities 文件静态验证测试
    - 验证 `gmail-channel.capabilities.json` 的类型、名称、白名单、密钥配置等字段
    - _需求: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7_

- [x] 11. 最终检查点 - 确保所有测试通过
  - ✅ 所有 53 个单元测试和属性测试通过
  - ✅ WASM 组件成功编译（release 模式）
  - ✅ 所有 12 个属性测试实现完成
  - ✅ 实现完成

## 备注

- 标记 `*` 的子任务为可选任务，可跳过以加速 MVP 开发
- 每个任务引用了具体的需求编号，确保可追溯性
- 属性测试验证设计文档中定义的正确性属性，使用 `proptest` 库
- 由于 WASM 沙箱限制，host API 调用（`channel_host::*`）需要通过 mock 或测试 harness 模拟
- 项目结构参考 `channels-src/feishu/`，保持一致的目录布局和构建流程
