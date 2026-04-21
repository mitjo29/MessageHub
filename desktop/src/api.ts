import { invoke } from "@tauri-apps/api/core";
import type {
  MessageRow,
  MessageDetail,
  ChannelInfo,
  UiConfig,
  Filter,
  SidebarCounts,
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
