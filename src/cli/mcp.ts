/**
 * MCP server entry point for tabctl.
 *
 * Usage: tabctl mcp [--profile <name>]
 *
 * Starts an MCP server on stdio that exposes browser tab management tools.
 * Agents connect via the standard MCP configuration:
 *
 *   {
 *     "mcpServers": {
 *       "tabctl": { "command": "tabctl", "args": ["mcp"] }
 *     }
 *   }
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { VERSION } from "./lib/constants";
import { registerTools } from "./lib/mcp-tools";

export async function startMcpServer(): Promise<void> {
  const server = new McpServer({
    name: "tabctl",
    version: VERSION,
  });

  registerTools(server);

  const transport = new StdioServerTransport();
  await server.connect(transport);
}
