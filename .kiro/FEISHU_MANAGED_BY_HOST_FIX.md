# Feishu managed_by_host 修复说明

## 问题描述

即使在 `feishu.capabilities.json` 中设置 `managed_by_host: false`，系统仍然会报错：
```
WARN Webhook secret required but not provided channel=feishu
```

## 根本原因

`managed_by_host` 标志没有被正确传递到路由器。具体来说：

1. 在 `src/channels/wasm/setup.rs` 中，`RegisteredEndpoint` 的 `require_secret` 标志被正确设置
2. 但是这个标志没有被存储在 `WasmChannelRouter` 中
3. 路由器的 `requires_secret()` 方法只检查 secret 是否存在于存储中，而不考虑 `require_secret` 标志
4. 结果是即使 `managed_by_host: false`，只要 secret 被配置，系统就会要求提供它

## 修复方案

### 1. 添加 `require_secrets` 字段到 `WasmChannelRouter`

在 `src/channels/wasm/router.rs` 中添加新字段来存储每个频道的 `require_secret` 标志：

```rust
pub struct WasmChannelRouter {
    // ... 其他字段 ...
    /// Whether secret validation is required by channel name.
    require_secrets: RwLock<HashMap<String, bool>>,
    // ... 其他字段 ...
}
```

### 2. 在 `register()` 方法中存储 `require_secret` 标志

```rust
pub async fn register(
    &self,
    channel: Arc<WasmChannel>,
    endpoints: Vec<RegisteredEndpoint>,
    secret: Option<String>,
    secret_header: Option<String>,
) {
    // ... 其他代码 ...
    
    // 存储 require_secret 标志
    let mut require_secrets = self.require_secrets.write().await;
    for endpoint in endpoints {
        require_secrets.insert(name.clone(), endpoint.require_secret);
        // ...
    }
}
```

### 3. 更新 `requires_secret()` 方法

改为检查 `require_secret` 标志而不是 secret 是否存在：

```rust
pub async fn requires_secret(&self, channel_name: &str) -> bool {
    self.require_secrets
        .read()
        .await
        .get(channel_name)
        .copied()
        .unwrap_or(false)
}
```

### 4. 在 `unregister()` 方法中清理标志

```rust
pub async fn unregister(&self, channel_name: &str) {
    // ... 其他代码 ...
    self.require_secrets.write().await.remove(channel_name);
    // ...
}
```

### 5. 添加测试

添加两个测试来验证修复：

- `test_managed_by_host_false_skips_secret_validation()` - 验证 `require_secret: false` 时不需要 secret
- `test_managed_by_host_true_requires_secret_validation()` - 验证 `require_secret: true` 时需要 secret

## 工作流程

现在的工作流程是：

1. **setup.rs** 中：
   - 如果 `managed_by_host: true`，设置 `host_webhook_secret = webhook_secret`
   - 如果 `managed_by_host: false`，设置 `host_webhook_secret = None`
   - `require_secret` 标志 = `host_webhook_secret.is_some()`

2. **router.rs** 中：
   - 存储 `require_secret` 标志到 `require_secrets` 字典
   - `requires_secret()` 方法检查这个标志而不是 secret 是否存在

3. **webhook_handler** 中：
   - 调用 `requires_secret()` 来决定是否需要验证 secret
   - 如果 `managed_by_host: false`，跳过 secret 验证
   - Feishu 频道代码自己负责验证请求的真实性

## 对 Feishu 的影响

- Feishu 频道现在可以正确地设置 `managed_by_host: false`
- 即使配置了 `feishu_verification_token` secret，系统也不会强制要求提供它
- Feishu 频道的 `is_authenticated_webhook()` 函数可以自己验证请求的真实性
- 不会再出现 "Webhook secret required but not provided" 的警告

## 修改的文件

- `src/channels/wasm/router.rs` - 核心修复
