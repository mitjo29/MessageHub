import { invoke } from "@tauri-apps/api/core";
import type { MessageRow, MessageDetail, ChannelInfo, UiConfig } from "./types";

export const api = {
  listMessages: (limit: number, offset: number) =>
    invoke<MessageRow[]>("list_messages", { limit, offset }),

  getMessage: (id: string) =>
    invoke<MessageDetail>("get_message", { id }),

  listChannels: () =>
    invoke<ChannelInfo[]>("list_channels"),

  getConfig: () =>
    invoke<UiConfig>("get_config"),
};
