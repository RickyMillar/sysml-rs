import * as vscode from "vscode";

/**
 * Status bar items for the language server:
 * 1. Server status (left): Starting / Running / Stopped / Error
 * 2. Library status (left): Loading / Loaded / Failed
 *
 * (Dependency + simulation indicators were part of the diagram/sim surface and
 * are dropped in the lightweight LSP-only extension; they come back when those
 * features do.)
 */
export class StatusBarManager implements vscode.Disposable {
  private readonly serverStatus: vscode.StatusBarItem;
  private readonly libraryStatus: vscode.StatusBarItem;

  constructor() {
    this.serverStatus = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      100,
    );
    this.serverStatus.name = "SysML Server";
    this.serverStatus.command = "sysml.showOutput";
    this.serverStatus.show();

    this.libraryStatus = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      99,
    );
    this.libraryStatus.name = "SysML Library";
    this.libraryStatus.command = "sysml.showOutput";

    this.setServerStopped();
  }

  // ---- Server status ----

  setServerStarting(): void {
    this.serverStatus.text = "$(loading~spin) SysML";
    this.serverStatus.tooltip = "SysML: Language server starting...";
    this.serverStatus.backgroundColor = undefined;
  }

  setServerRunning(): void {
    this.serverStatus.text = "$(check) SysML";
    this.serverStatus.tooltip = "SysML: Language server running";
    this.serverStatus.backgroundColor = undefined;
  }

  setServerStopped(): void {
    this.serverStatus.text = "$(circle-slash) SysML";
    this.serverStatus.tooltip = "SysML: Language server stopped";
    this.serverStatus.backgroundColor = undefined;
  }

  setServerError(message: string): void {
    this.serverStatus.text = "$(error) SysML";
    this.serverStatus.tooltip = `SysML: Error - ${message}`;
    this.serverStatus.backgroundColor = new vscode.ThemeColor(
      "statusBarItem.errorBackground",
    );
  }

  // ---- Library status ----

  setLibraryLoading(): void {
    this.libraryStatus.text = "$(loading~spin) Library";
    this.libraryStatus.tooltip = "SysML: Loading standard library...";
    this.libraryStatus.show();
  }

  setLibraryLoaded(fileCount: number): void {
    this.libraryStatus.text = "$(library) Library";
    this.libraryStatus.tooltip = `SysML: Standard library loaded (${fileCount} files)`;
    this.libraryStatus.show();
  }

  setLibraryFailed(message: string): void {
    this.libraryStatus.text = "$(warning) Library";
    this.libraryStatus.tooltip = `SysML: Library load failed - ${message}`;
    this.libraryStatus.backgroundColor = new vscode.ThemeColor(
      "statusBarItem.warningBackground",
    );
    this.libraryStatus.show();
  }

  hideLibraryStatus(): void {
    this.libraryStatus.hide();
  }

  // ---- Disposal ----

  dispose(): void {
    this.serverStatus.dispose();
    this.libraryStatus.dispose();
  }
}
