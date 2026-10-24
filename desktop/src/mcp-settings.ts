import type { AtlasApi } from "./atlas-api";
import type { McpConnectionStatus } from "./types";

export type McpSettingsApi = Pick<AtlasApi, "listMcpConnections" | "connectMcpServer" | "disconnectMcpServer" | "cancelMcpConnection">;
type Operation = "connecting" | "disconnecting";

/** Own async state and redraw only this panel, preserving the model Settings form. */
export function createMcpSettingsPanel(api: McpSettingsApi): HTMLElement {
  const panel = document.createElement("section");
  panel.className = "mcp-settings";
  panel.setAttribute("aria-label", "MCP connections");
  let connections: McpConnectionStatus[] = [];
  let loading = false;
  let refreshAgain = false;
  let discoveryError = "";
  const busy = new Map<string, Operation>();
  const cancelling = new Set<string>();
  const errors = new Map<string, string>();
  const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);
  const makeButton = (text: string, action: () => void, disabled = false) => {
    const button = document.createElement("button");
    button.type = "button"; button.className = "secondary-button";
    button.textContent = text; button.disabled = disabled;
    button.addEventListener("click", action);
    return button;
  };
  const alert = (message: string) => {
    const element = document.createElement("p"); element.className = "settings-error";
    element.setAttribute("role", "alert"); element.textContent = message; return element;
  };
  const draw = () => {
    const heading = document.createElement("h2"); heading.textContent = "MCP connections";
    const hint = document.createElement("p"); hint.className = "settings-hint";
    hint.textContent = "Configure servers in your user-local mcp.toml. Connect opens your browser to authorize access; credentials stay in your OS credential store. Changes apply immediately.";
    panel.replaceChildren(heading, hint, makeButton(loading ? "Loading connections…" : "Refresh connections", () => void refresh(), loading));
    if (discoveryError) panel.append(alert(discoveryError));
    if (!loading && !discoveryError && !connections.length) {
      const empty = document.createElement("p"); empty.textContent = "No MCP servers configured."; panel.append(empty);
    }
    for (const connection of connections) {
      const row = document.createElement("div"); row.className = "mcp-connection";
      const label = document.createElement("strong"); label.textContent = connection.name;
      const status = document.createElement("span"); status.setAttribute("role", "status");
      const operation = busy.get(connection.name);
      status.textContent = operation === "connecting" ? "Waiting for browser authorization…" : operation === "disconnecting" ? "Disconnecting…" : connection.error ? "Connection needs attention" : connection.oauth ? (connection.connected ? "Connected" : "Not connected") : "No OAuth connection required";
      const transport = document.createElement("span"); transport.className = "settings-hint";
      transport.textContent = connection.transport === "http" ? "HTTP" : "Standard input/output";
      row.append(label, transport, status);
      if (operation === "connecting") row.append(makeButton(cancelling.has(connection.name) ? "Cancelling…" : "Cancel connection", () => void cancel(connection.name), cancelling.has(connection.name)));
      else if (connection.oauth && connection.error) {
        row.append(
          makeButton("Connect", () => void run(connection, false), !!operation),
          makeButton("Disconnect", () => void run(connection, true), !!operation),
        );
      } else if (connection.oauth) row.append(makeButton(connection.connected ? "Disconnect" : "Connect", () => void run(connection), !!operation));
      if (connection.error) row.append(alert(connection.error));
      const error = errors.get(connection.name); if (error) row.append(alert(error));
      panel.append(row);
    }
  };
  const refresh = async () => {
    if (loading) { refreshAgain = true; return; }
    loading = true; discoveryError = ""; draw();
    try { connections = await api.listMcpConnections(); }
    catch (error) { discoveryError = errorText(error); }
    finally {
      loading = false; draw();
      if (refreshAgain) { refreshAgain = false; void refresh(); }
    }
  };
  const run = async (connection: McpConnectionStatus, disconnect = connection.connected) => {
    const name = connection.name; if (busy.has(name)) return;
    busy.set(name, disconnect ? "disconnecting" : "connecting"); errors.delete(name); draw();
    try {
      if (disconnect) await api.disconnectMcpServer(name);
      else await api.connectMcpServer(name);
    } catch (error) { if (!cancelling.has(name)) errors.set(name, errorText(error)); }
    finally {
      busy.delete(name); cancelling.delete(name); draw(); await refresh();
    }
  };
  const cancel = async (name: string) => {
    if (cancelling.has(name)) return;
    cancelling.add(name); errors.delete(name); draw();
    try { await api.cancelMcpConnection(name); }
    catch (error) { cancelling.delete(name); errors.set(name, errorText(error)); draw(); }
    // Keep the row busy until the original connect worker has exited.
  };
  void refresh();
  return panel;
}
