# Gmail Channel for IronClaw

A WASM-based channel that integrates Gmail with IronClaw, enabling automated email notification processing and summarization.

## Features

- **Push Notifications**: Receives Gmail notifications via Google Pub/Sub
- **Deduplication**: Prevents duplicate processing using history IDs and message IDs
- **Smart Summarization**: Uses the `gmail-summarize` skill to create organized email summaries
- **Workspace Integration**: Saves summaries to the workspace for persistent storage and search
- **Multi-language Support**: Configurable summary language (default: Chinese)
- **User Isolation**: Supports multiple users with separate email accounts

## Architecture

```
Gmail API (Pub/Sub)
        ↓
Gmail Channel (WASM)
        ↓
Agent + gmail-summarize Skill
        ↓
Workspace (Database)
```

### Components

1. **Gmail Channel** (`src/lib.rs`)
   - Receives Pub/Sub notifications
   - Validates and deduplicates messages
   - Emits structured events to the Agent

2. **gmail-summarize Skill** (`skills/gmail-summarize/SKILL.md`)
   - Provides workflow guidance for email processing
   - Defines summary format and best practices
   - Guides the Agent through the summarization process

3. **Workspace Storage**
   - Persists summaries in the database
   - Enables full-text and semantic search
   - Maintains version history

## Quick Start

### Installation

```bash
cd channels-src/gmail
bash build.sh
```

This will:
1. Build the Gmail channel WASM component
2. Install the `gmail-summarize` skill to `~/.ironclaw/skills/gmail-summarize/`
3. Display installation instructions

Then follow the on-screen instructions to:
1. Copy the WASM channel to `~/.ironclaw/channels/`
2. Verify the skill installation
3. Configure Gmail Pub/Sub subscription
4. Restart IronClaw

### Configuration

```json
{
  "pubsub_subscription": "projects/your-project/subscriptions/your-subscription",
  "user_id": "your-ironclaw-user-id",
  "summary_language": "Chinese"
}
```

## Workflow

### 1. Gmail Notification Received

```
Gmail API → Pub/Sub Topic → HTTP Webhook → Gmail Channel
```

### 2. Channel Processing

- Validates Pub/Sub message format
- Decodes Base64URL payload
- Checks for duplicates (history ID and message ID)
- Emits notification to Agent

### 3. Agent Processing

- Receives notification with skill reference
- Loads `gmail-summarize` skill from system prompt
- Follows skill guidance to:
  - Retrieve new messages using Gmail tool
  - Analyze email content
  - Create formatted summary
  - Save to workspace using memory_write tool

### 4. Summary Storage

```
Workspace Database
├── memory_documents
│   └── path: summaries/gmail/2024-01-15T10-30-45-123456Z.md
│       content: {formatted summary}
│       user_id: {user}
│       created_at: {timestamp}
└── memory_chunks (for search indexing)
```

## Configuration

### Channel Configuration

**pubsub_subscription** (optional)
- Expected Pub/Sub subscription name for source verification
- If set, notifications from other subscriptions are rejected
- Example: `projects/my-project/subscriptions/gmail-notifications`

**user_id** (optional)
- IronClaw user ID for all emitted messages
- If not set, uses the email address from the notification
- Useful for routing all Gmail notifications to a specific user

**summary_language** (optional)
- Language for email summarization
- Default: `Chinese`
- Supported: `English`, `Spanish`, `French`, etc.
- Passed to the Agent to guide summary language

### Environment Variables

```bash
# Gmail API credentials
export GMAIL_CLIENT_ID="your-client-id.apps.googleusercontent.com"
export GMAIL_CLIENT_SECRET="your-client-secret"
export GMAIL_REFRESH_TOKEN="your-refresh-token"

# Channel configuration
export GMAIL_PUBSUB_SUBSCRIPTION="projects/your-project/subscriptions/your-subscription"
export GMAIL_USER_ID="your-ironclaw-user-id"
export GMAIL_SUMMARY_LANGUAGE="Chinese"
```

## API Reference

### Incoming Message Format

```json
{
  "message": {
    "data": "base64url-encoded-notification",
    "messageId": "unique-message-id",
    "publishTime": "2024-01-15T10:30:45.123456Z"
  },
  "subscription": "projects/your-project/subscriptions/your-subscription"
}
```

### Notification Data (decoded)

```json
{
  "emailAddress": "user@example.com",
  "historyId": 24430
}
```

### Emitted Message to Agent

```
New Gmail notification for user@example.com.
Use the gmail-summarize skill to process this notification.

Details:
- Email: user@example.com
- History ID: 24430
- Summary Language: Chinese
- Save Path: summaries/gmail/2024-01-15T10-30-45-123456Z.md
```

## Deduplication

The channel implements two-level deduplication:

### 1. History ID Deduplication
- Tracks the latest history ID processed
- Rejects notifications with history ID ≤ stored value
- Prevents reprocessing of old emails

### 2. Message ID Deduplication
- Maintains a FIFO queue of recent message IDs (max 200)
- Rejects duplicate Pub/Sub messages
- Handles Pub/Sub retry scenarios

## Workspace Integration

### Summary Storage Path

```
summaries/gmail/{timestamp}.md
```

Example: `summaries/gmail/2024-01-15T10-30-45-123456Z.md`

### Summary Format

```markdown
# Gmail 邮件摘要

**邮箱**: user@example.com
**History ID**: 24430
**通知时间**: 2024-01-15T10:30:45Z

## 邮件列表

### 1. Project Alpha - Status Update
- **发件人**: manager@company.com
- **时间**: 2024-01-15 10:15 AM
- **摘要**: Weekly status update on Project Alpha
- **行动项**: Review Q1 roadmap

## 关键要点

- Project Alpha is on schedule
- Q1 roadmap needs review

## 待办事项

- [ ] Review Q1 roadmap
```

### Accessing Summaries

```bash
# List all Gmail summaries
ironclaw workspace list summaries/gmail/

# Read a specific summary
ironclaw workspace read summaries/gmail/2024-01-15T10-30-45-123456Z.md

# Search summaries
ironclaw workspace search "Project Alpha"
```

## Skill: gmail-summarize

The `gmail-summarize` skill provides:

1. **Workflow Steps**
   - Retrieve new messages
   - Analyze content
   - Create summary
   - Save to workspace

2. **Best Practices**
   - Summary quality guidelines
   - Organization standards
   - Language consistency

3. **Format Templates**
   - Markdown structure
   - Section organization
   - Example summaries

4. **Troubleshooting**
   - Common errors
   - Solutions
   - Verification steps

See [skills/gmail-summarize/SKILL.md](skills/gmail-summarize/SKILL.md) for full details.

## Error Handling

### Common Errors

**Invalid Pub/Sub Message**
- Cause: Malformed JSON or missing required fields
- Solution: Verify Pub/Sub message format

**Base64 Decode Error**
- Cause: Invalid Base64URL encoding
- Solution: Check Gmail API notification format

**Duplicate Message**
- Cause: Pub/Sub retry or duplicate notification
- Solution: Normal behavior, message is skipped

**Stale History ID**
- Cause: Notification for already-processed emails
- Solution: Normal behavior, message is skipped

**Gmail Tool Error**
- Cause: Invalid credentials or API error
- Solution: Verify Gmail API credentials and permissions

**Memory Write Error**
- Cause: Workspace storage issue
- Solution: Check workspace availability and permissions

## Testing

### Manual Testing

1. Send a test email to the configured Gmail account
2. Verify Pub/Sub notification is received
3. Check Agent processes the notification
4. Verify summary is saved to workspace

### Debugging

```bash
# View channel logs
ironclaw logs grep "gmail"

# Check workspace for summaries
ironclaw workspace list summaries/gmail/

# Verify skill is loaded
ironclaw skills info gmail-summarize
```

## Performance

- **Notification Processing**: < 1 second
- **Email Retrieval**: Depends on Gmail API response time
- **Summary Generation**: Depends on LLM response time
- **Workspace Storage**: < 100ms

## Limitations

- **Batch Size**: Processes one notification at a time
- **History Depth**: Limited by Gmail API (typically last 1 year)
- **Summary Size**: Limited by workspace storage (typically 1MB per file)
- **Rate Limiting**: Subject to Gmail API quotas

## Security

- **Credentials**: Stored in IronClaw secrets store
- **Deduplication**: Prevents replay attacks
- **Workspace Isolation**: User-scoped storage
- **WASM Sandbox**: Isolated execution environment

## Troubleshooting

See [INSTALLATION.md](INSTALLATION.md#troubleshooting) for detailed troubleshooting steps.

## Development

### Building from Source

```bash
cd channels-src/gmail
cargo build --release --target wasm32-wasip2
```

### Testing

```bash
cargo test
```

### Code Structure

```
src/
├── lib.rs                    # Main channel implementation
├── types.rs                  # Data structures
└── handlers.rs               # Message handlers

skills/
└── gmail-summarize/
    └── SKILL.md              # Skill definition

tests/
└── integration_tests.rs      # Integration tests
```

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

## License

MIT

## Support

For issues or questions:

1. Check [INSTALLATION.md](INSTALLATION.md#troubleshooting)
2. Review the skill documentation
3. Check IronClaw logs
4. Consult the Gmail API documentation

## Related Resources

- [IronClaw Documentation](https://docs.ironclaw.ai)
- [Gmail API Documentation](https://developers.google.com/gmail/api)
- [Google Pub/Sub Documentation](https://cloud.google.com/pubsub/docs)
- [WASM Component Model](https://component-model.bytecodealliance.org/)
