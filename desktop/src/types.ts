export type MessageRow = {
  id: string;
  timestamp: string;
  channel: string;
  channelLabel: string | null;
  senderName: string;
  subject: string | null;
  preview: string;
  category: string | null;
  priority: number | null;
  isRead: boolean;
};

export type AttachmentInfo = {
  filename: string;
  sizeBytes: number;
};

export type MessageDetail = MessageRow & {
  body: string;
  html: string | null;
  threadId: string;
  attachments: AttachmentInfo[];
};

export type ChannelInfo = {
  id: string;
  channelType: string;
  label: string;
  enabled: boolean;
  status: string;
  lastSyncAt: string | null;
};

export type UiConfig = {
  dbPath: string;
  channelCount: number;
};

export type Filter =
  | { kind: "all" }
  | { kind: "unread" }
  | { kind: "priorityHigh" }
  | { kind: "channel"; channelType: string };

export type ChannelCount = {
  channelType: string;
  total: number;
  unread: number;
};

export type SidebarCounts = {
  all: number;
  unread: number;
  priorityHigh: number;
  byChannel: ChannelCount[];
};

export interface ReplyDraft {
  threadId: string;
  inReplyToMessageId: string;
  body: string;
  subject: string | null;
  updatedAt: string;
}

export interface AiDraft {
  draftId: string;
  body: string;
  confidence: number;
}

export interface AiDraftSummary {
  id: string;
  createdAt: string;
  confidence: number;
  preview: string;
  body: string;
  hasUserEdit: boolean;
}

export interface CloudStatus {
  configured: boolean;
  model: string | null;
}
