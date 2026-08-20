import * as vscode from "vscode";
import { SysmlLanguageClient } from "./client";
import { StatusBarManager } from "./statusBar";
import { installServer } from "./serverBinary";

/**
 * Lightweight SysML/KerML VS Code extension — language server client only.
 *
 * Provides: syntax highlighting (TextMate grammars), snippets, and the full
 * LSP feature set (diagnostics, hover, completion, go-to-definition, semantic
 * tokens, inlay hints, …) by launching the `sysml-lsp-server` binary.
 *
 * The diagram webview, simulation panels, and debug adapter were removed in the
 * Bucket 3.1 renderer rework; they return once the React-SVG renderer ships.
 */

let client: SysmlLanguageClient | undefined;
let statusBar: StatusBarManager | undefined;

export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  const outputChannel = vscode.window.createOutputChannel("SysML", {
    log: true,
  }) as vscode.LogOutputChannel;
  context.subscriptions.push(outputChannel);

  statusBar = new StatusBarManager();
  context.subscriptions.push(statusBar);

  // Commands: show output, restart, install server.
  context.subscriptions.push(
    vscode.commands.registerCommand("sysml.showOutput", () =>
      outputChannel.show(),
    ),
    vscode.commands.registerCommand("sysml.restartServer", async () => {
      if (!client) {
        return;
      }
      statusBar?.setServerStarting();
      try {
        await client.restart();
        statusBar?.setServerRunning();
      } catch (err) {
        statusBar?.setServerError(errMsg(err));
      }
    }),
    vscode.commands.registerCommand("sysml.installServer", async () => {
      const installed = await installServer(context);
      if (installed && client) {
        await vscode.commands.executeCommand("sysml.restartServer");
      }
    }),
  );

  try {
    client = new SysmlLanguageClient(context, outputChannel);
  } catch (err) {
    outputChannel.error(`Failed to create language client: ${errMsg(err)}`);
    statusBar.setServerError(errMsg(err));
    return;
  }

  // Library status → status bar.
  client.onLibraryStatus((status) => {
    switch (status.phase) {
      case "loading":
        statusBar?.setLibraryLoading();
        break;
      case "loaded":
        statusBar?.setLibraryLoaded(status.fileCount ?? 0);
        break;
      case "failed":
        statusBar?.setLibraryFailed(status.message ?? "Unknown error");
        break;
    }
  });

  // Server state → status bar.
  client.onStateChange((_oldState, newState) => {
    if (newState === "running") {
      statusBar?.setServerRunning();
    } else if (newState === "stopped") {
      statusBar?.setServerStopped();
      statusBar?.hideLibraryStatus();
    }
  });

  // Restart on server-config changes.
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (
        e.affectsConfiguration("sysml.server.path") ||
        e.affectsConfiguration("sysml.server.extraArgs")
      ) {
        const action = await vscode.window.showInformationMessage(
          "SysML server configuration changed. Restart the server?",
          "Restart",
          "Later",
        );
        if (action === "Restart") {
          await vscode.commands.executeCommand("sysml.restartServer");
        }
      }
    }),
  );

  statusBar.setServerStarting();
  try {
    await client.start();
    statusBar.setServerRunning();
    outputChannel.info("SysML extension activated");
  } catch (err) {
    outputChannel.error(`Failed to start language server: ${errMsg(err)}`);
    statusBar.setServerError(errMsg(err));
    vscode.window.showErrorMessage(
      `SysML language server failed to start: ${errMsg(err)}`,
    );
  }
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}

function errMsg(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
