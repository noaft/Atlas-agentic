import "./styles.css";
import { createTauriAtlasApi } from "./atlas-api";
import { ChatStore } from "./chat-state";
import { mountChat } from "./chat-view";

const root = document.querySelector<HTMLElement>("#app");

if (root) {
  const store = new ChatStore(createTauriAtlasApi());
  const unmount = mountChat(root, store);
  void store.initialize();
  window.addEventListener("beforeunload", unmount, { once: true });
}
