import { invoke } from "@tauri-apps/api/core";
import type {
  MessageRow,
  MessageDetail,
  ChannelInfo,
  UiConfig,
  Filter,
  SidebarCounts,
  ReplyDraft,
  AiDraft,
  AiDraftSummary,
  CloudStatus,
} from "./types";

export const api = {
  listMessages: (filter: Filter, limit: number, offset: number) =>
    invoke<MessageRow[]>("list_messages", { filter, limit, offset }),

  getMessage: (id: string) =>
    invoke<MessageDetail>("get_message", { id }),

  listChannels: () =>
    invoke<ChannelInfo[]>("list_channels"),

  getConfig: () =>
    invoke<UiConfig>("get_config"),

  markRead: (id: string, read: boolean) =>
    invoke<void>("mark_read", { id, read }),

  sidebarCounts: () =>
    invoke<SidebarCounts>("sidebar_counts"),
};

export function saveReplyDraft(
  threadId: string,
  inReplyToMessageId: string,
  body: string,
  subject: string | null,
): Promise<void> {
  return invoke("save_reply_draft", {
    threadId,
    inReplyToMessageId,
    body,
    subject,
  });
}

export function getReplyDraft(threadId: string): Promise<ReplyDraft | null> {
  return invoke("get_reply_draft", { threadId });
}

export function deleteReplyDraft(threadId: string): Promise<void> {
  return invoke("delete_reply_draft", { threadId });
}

export function sendEmailReply(
  threadId: string,
  inReplyToMessageId: string,
  body: string,
  subject: string,
): Promise<void> {
  return invoke("send_email_reply", {
    threadId,
    inReplyToMessageId,
    body,
    subject,
  });
}

export function aiDraftReply(
  messageId: string,
  redact: boolean,
): Promise<AiDraft> {
  return invoke("ai_draft_reply", { messageId, redact });
}

export function listAiDrafts(messageId: string): Promise<AiDraftSummary[]> {
  return invoke("list_ai_drafts", { messageId });
}

export function cloudConfigStatus(): Promise<CloudStatus> {
  return invoke("cloud_config_status");
}
