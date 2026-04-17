# 需求文档

## 简介

Gmail Channel 是一个新的 WASM channel 插件，用于接收 Gmail 邮件通知事件（通过 Google Pub/Sub 推送），并将通知事件发送给 agent。Agent 使用已有的 Gmail tool 获取邮件内容，分析总结后通过已配置的输出 channel 将结果推送给用户。输出 channel 是可配置的，用户可以指定任意已安装的 channel（如 feishu、slack、telegram 等）作为输出目标；如果未配置输出 channel，则将分析结果保存到工作区的 gmail 目录下。

该 channel 与现有的 Gmail tool（用于主动读写邮件）不同，Gmail Channel 是一个被动接收事件的通道：当用户收到新邮件时，Google Pub/Sub 会向配置的 webhook 端点推送通知，channel 解析通知事件后 emit 给 agent，由 agent 通过 Gmail tool 获取邮件内容、总结分析，再通过输出 channel 推送给用户。

**核心工作流程：**
1. Gmail Channel 收到 Google Pub/Sub 推送通知
2. Gmail Channel 解析通知，提取基本信息（email 地址、History ID）
3. Gmail Channel 将通知事件 emit 给 agent（包含 email 地址、History ID 等元信息）
4. Agent 使用已有的 Gmail tool 获取邮件内容
5. Agent 分析总结邮件内容
6. Agent 将结果通过配置的 output channel 推送给用户

## 术语表

- **Gmail_Channel**: 接收 Gmail 推送通知并将通知事件转发给 agent 的 WASM channel 插件，不直接调用 Gmail API 读取邮件内容
- **Google_Pub_Sub**: Google Cloud 的消息发布/订阅服务，Gmail 使用该服务推送邮件变更通知
- **Push_Notification**: Google Pub/Sub 向 webhook 端点发送的 HTTP POST 请求，包含 base64 编码的通知数据
- **Notification_Event**: Gmail_Channel 解析 Push_Notification 后生成的结构化事件，包含 email 地址和 History_ID，通过 emit 发送给 Agent
- **History_ID**: Gmail 中标识邮箱状态的递增数字，Agent 可使用该 ID 通过 Gmail tool 增量获取新邮件
- **Gmail_Tool**: IronClaw 中已有的 Gmail 工具，Agent 使用该工具调用 Gmail API 读取邮件内容
- **Gmail_API**: Google 提供的 RESTful API，用于读取邮件内容、管理邮箱等操作（由 Gmail_Tool 调用，非 Gmail_Channel 直接调用）
- **OAuth_Token**: 通过 Google OAuth 2.0 获取的访问令牌，用于调用 Gmail API
- **Agent**: IronClaw 的核心处理引擎，接收 channel 发出的通知事件，使用 Gmail_Tool 获取邮件内容并进行分析处理
- **Output_Channel**: 用于将 agent 处理结果推送给用户的可配置输出通道，可以是任意已安装的 channel（如 feishu、slack、telegram 等）
- **Summary_Storage**: 当未配置 Output_Channel 时，用于在工作区 `channels/gmail/summaries/` 目录下保存分析结果的本地存储机制
- **Webhook_Endpoint**: Gmail Channel 注册的 HTTP 端点，用于接收 Google Pub/Sub 推送通知
- **Capabilities_File**: 定义 channel 权限、HTTP 白名单、密钥配置等的 JSON 文件

## 需求

### 需求 1: Channel 初始化与配置

**用户故事：** 作为用户，我希望能够配置 Gmail channel 的 Google Pub/Sub 订阅设置，以便系统可以接收 Gmail 推送通知。

#### 验收标准

1. 当 Gmail_Channel 在启动时收到配置 JSON，Gmail_Channel 应解析配置，包括可选的 owner_id、可选的 output_channel、必需的 pubsub_topic（Pub/Sub topic 全名）和可选的 label_ids（监听的 Gmail 标签列表，默认为 `["INBOX"]`）
2. 当 Gmail_Channel 启动成功后，Gmail_Channel 应注册一个 webhook 端点 `/webhook/gmail`，接受 POST 请求
3. 当 Gmail_Channel 启动时，Gmail_Channel 应使用配置的 pubsub_topic 和 label_ids 调用 Gmail API `watch` 端点，为配置的邮箱建立 Pub/Sub 订阅
4. 如果启动时 Gmail API `watch` 调用失败，Gmail_Channel 应记录警告日志并继续启动，允许在下一个轮询周期重新建立订阅
5. Gmail_Channel 应将当前 History_ID 持久化到工作区存储中，用于跨回调调用的通知状态跟踪
6. 当配置了 output_channel 时，Gmail_Channel 应验证 output_channel 值引用的是一个已知的 channel 名称（如 "feishu"、"slack" 或 "telegram"）

### 需求 2: 接收并解析 Google Pub/Sub 推送通知

**用户故事：** 作为用户，我希望系统能够实时接收和解析 Gmail 推送通知，以便及时检测到新邮件事件。

#### 验收标准

1. 当 POST 请求到达 `/webhook/gmail` 端点时，Gmail_Channel 应将请求体解析为 Google Pub/Sub 推送消息
2. 当 Push_Notification 包含有效的 base64 编码数据载荷时，Gmail_Channel 应解码载荷并提取 email 地址和 History_ID
3. 当解码后的通知引用的 History_ID 大于已存储的 History_ID 时，Gmail_Channel 应接受该通知作为新事件进行处理
4. 如果 Push_Notification 请求体格式错误或缺少必要字段，Gmail_Channel 应返回 HTTP 200 确认接收并记录错误日志（返回非 200 会导致 Pub/Sub 重试）
5. 当 Push_Notification 处理成功后，Gmail_Channel 应返回 HTTP 200 和空 JSON 响应体

### 需求 3: 将通知事件发送给 Agent

**用户故事：** 作为用户，我希望 Gmail channel 将通知事件转发给 agent，以便 agent 可以使用已有的 Gmail tool 获取和分析邮件内容。

#### 验收标准

1. 当检测到有效的新通知事件时，Gmail_Channel 应向 Agent emit 一条 Notification_Event 消息，包含 Push_Notification 中的 email 地址和 History_ID
2. Gmail_Channel 应以结构化 prompt 格式化 emit 的消息，指示 Agent 使用 Gmail_Tool 根据提供的 History_ID 获取新邮件
3. 当 emit Notification_Event 时，Gmail_Channel 应包含元数据 JSON，包括 email 地址、History_ID 和通知时间戳，用于审计和路由
4. 当 emit Notification_Event 时，Gmail_Channel 应使用配置的 owner user ID 作为 user_id 字段，确保消息路由到正确的用户会话
5. 如果未配置 owner_id，Gmail_Channel 应使用通知中的 email 地址作为 user_id
6. 当通知事件成功 emit 后，Gmail_Channel 应将存储的 History_ID 更新为通知中的值，防止重复处理

### 需求 4: Agent 响应处理与结果输出

**用户故事：** 作为用户，我希望 agent 的分析结果能够通过可配置的输出 channel 推送，以便我可以在首选的消息平台接收摘要，或在未配置输出 channel 时将结果保存到本地。

#### 验收标准

1. 当 agent 产生响应时，Gmail_Channel 应通过 `on_respond` 回调接受响应
2. 当配置了 output_channel 时，Gmail_Channel 应在 emit 的消息元数据中包含 output_channel 名称，指示 agent 或路由层将响应转发到指定的 Output_Channel
3. 如果未配置 output_channel，Gmail_Channel 应将 agent 响应保存为工作区 `channels/gmail/summaries/` 下的摘要文件，文件名由通知时间戳和 History_ID 生成
4. 当本地保存摘要文件时，Gmail_Channel 应格式化文件内容，包含通知元数据（email 地址、History_ID）和 agent 的分析结果
5. 当通过 `on_broadcast` 收到广播消息时，Gmail_Channel 应记录广播日志用于审计
6. 无论输出路由方式如何，Gmail_Channel 应记录所有响应内容用于审计

### 需求 5: Pub/Sub 订阅续期

**用户故事：** 作为用户，我希望 Gmail 推送订阅能够自动续期，以便无需手动干预即可持续接收通知。

#### 验收标准

1. 当 Gmail_Channel 在 `on_http_request` 中处理 Pub/Sub 通知时，Gmail_Channel 应检查工作区中存储的订阅过期时间戳
2. 当订阅过期时间在 24 小时内时，Gmail_Channel 应调用 Gmail API `watch` 端点续期订阅
3. 当订阅续期成功后，Gmail_Channel 应将新的过期时间戳持久化到工作区存储
4. 如果订阅续期失败，Gmail_Channel 应记录错误日志并在下一次收到 webhook 请求时重试

### 需求 6: 安全与权限控制

**用户故事：** 作为用户，我希望 Gmail channel 能够执行访问控制，以确保只有授权的通知被处理。

#### 验收标准

1. Gmail_Channel 应通过检查消息属性中的订阅名称，验证传入的 Pub/Sub 通知来源于预期的订阅
2. 当配置了 owner_id 时，Gmail_Channel 应使用 owner_id 作为 emit 消息的 user_id，确保消息路由到正确的用户会话
3. Gmail_Channel 应在 capabilities 文件中定义 HTTP 白名单，仅限 `gmail.googleapis.com` 的 `watch` API 调用（Gmail API 邮件读取调用由 Gmail_Tool 处理，非 channel 直接调用）
4. Gmail_Channel 应使用主机的凭证注入机制获取 OAuth token，确保 WASM 模块无法直接访问密钥

### 需求 7: 去重与幂等处理

**用户故事：** 作为用户，我希望重复通知能够被优雅处理，以确保同一通知事件不会被多次处理。

#### 验收标准

1. Gmail_Channel 应在工作区存储中维护最新已处理的 History_ID 作为主要去重机制
2. 当通知中的 History_ID 小于或等于已存储的 History_ID 时，Gmail_Channel 应跳过该通知并返回 HTTP 200
3. Gmail_Channel 应在工作区存储中维护一组最近处理过的 Pub/Sub 消息 ID，用于处理完全重复的投递
4. 当 Pub/Sub 消息 ID 在已处理集合中被找到时，Gmail_Channel 应跳过处理并记录 debug 日志
5. Gmail_Channel 应将已处理的 Pub/Sub 消息 ID 集合限制为最近 200 条，以限制存储使用量

### 需求 8: Capabilities 文件定义

**用户故事：** 作为开发者，我希望有一个定义良好的 capabilities 文件，以便主机运行时能够正确配置权限和路由。

#### 验收标准

1. Capabilities_File 应声明 channel 类型为 "channel"，名称为 "gmail"
2. Capabilities_File 应定义 OAuth 认证配置，包括 Google OAuth 2.0 授权端点、token 端点和 `gmail.readonly` 权限范围，与 Gmail tool 使用相同的认证模式
3. Capabilities_File 应定义 HTTP 白名单，仅允许向 `gmail.googleapis.com` 发送 POST 请求，路径前缀为 `/gmail/v1/users/me/watch`，仅用于 Pub/Sub 订阅管理
4. Capabilities_File 应配置 webhook 端点 `/webhook/gmail`，轮询设置为禁用（`allow_polling: false`）
5. Capabilities_File 应定义凭证注入，使用 `google_oauth_token` 密钥，以 bearer token 方式注入到 `gmail.googleapis.com` 主机
6. Capabilities_File 应定义 setup 部分，包含 `google_oauth_client_id` 和 `google_oauth_client_secret` 两个必需密钥
7. Capabilities_File 应设置 workspace_prefix 为 `channels/gmail/`，将所有工作区存储操作限定在 gmail 目录下
