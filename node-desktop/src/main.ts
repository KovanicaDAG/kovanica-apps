import { invoke } from "@tauri-apps/api/core";
import { mountNodeApp } from "./ui.ts";

mountNodeApp({
  invoke: (command, args) => invoke(command, args),
  document,
  window,
});
