---
name: gmail-summarize
description: Analyze and summarize Gmail notifications
keywords: [gmail, email, summarize, notification, workspace]
license: MIT
---

# Gmail Summarization Skill

When triggered with a Gmail notification, follow these steps:

## Workflow

1. **List unread emails**
   ```
   gmail {"action": "list_messages", "query": "is:unread", "max_results": 5}
   ```

2. **Retrieve each message**
   ```
   gmail {"action": "get_message", "message_id": {id}}
   ```
   Extract: From, Subject, Date, Body, Attachments

3. **Analyze content**
   - Action items (tasks needing response)
   - Deadlines (time-sensitive info)
   - Priority level (high/medium/low)

4. **Create summary** in markdown format:
   ```markdown
   # Gmail 邮件摘要
   **邮箱**: {email}
   **时间**: {timestamp}
   
   ## 邮件列表
   ### 1. {Subject}
   - **发件人**: {From}
   - **优先级**: {Priority}
   - **摘要**: {Summary}
   - **行动项**: {Actions}
   
   ## 待办事项
   - [ ] {Action 1}
   - [ ] {Action 2}
   ```

5. **Save to workspace**
   ```
   memory_write: target="summaries/gmail/{timestamp}.md"
   ```

## Supported Gmail Actions

   - `list_messages`
   - `get_message`
   - `send_message`
   - `create_draft`
   - `reply_to_message`
   - `trash_message`

## Tips

- Sort by priority (high → medium → low)
- Extract specific action items and deadlines
- Use consistent formatting
- Include sender information for context
