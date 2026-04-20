export type MessageRow = {
  id: string;
  timestamp: string;
  channel: string;
  channel_label: string | null;
  sender_name: string;
  subject: string | null;
  preview: string;
  category: string | null;
  priority: number | null;
  is_read: boolean;
};

export type AttachmentInfo = {
  filename: string;
  size_bytes: number;
};

export type MessageDetail = MessageRow & {
  body: string;
  html: string | null;
  thread_id: string;
  attachments: AttachmentInfo[];
};

export type ChannelInfo = {
  id: string;
  channel_type: string;
  label: string;
  enabled: boolean;
  status: string;
  last_sync_at: string | null;
};

export type UiConfig = {
  db_path: string;
  channel_count: number;
};
