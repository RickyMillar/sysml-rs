import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
    State,
} from 'vscode-languageclient/node';
import { findServerBinary } from './serverBinary';

/**
 * Initialization options sent to the server, matching FeatureFlags in types.rs.
 */
interface ServerInitOptions {
    resolution: boolean;
    validation: boolean;
    resolutionTimeoutMs: number;
    libraryPath: string;
    maxIndexFiles: number;
    inlayHints: boolean;
}

/**
 * Wraps the vscode-languageclient LanguageClient with SysML-specific
 * configuration, middleware, and lifecycle management.
 */
export class SysmlLanguageClient implements vscode.Disposable {
    private client: LanguageClient | undefined;
    private readonly context: vscode.ExtensionContext;
    private readonly outputChannel: vscode.LogOutputChannel;
    private serverBinaryPath: string | undefined;
    private readonly disposables: vscode.Disposable[] = [];
    private stateChangeCallback?: (oldState: string, newState: string) => void;
    private libraryStatusCallback?: (status: { phase: string; fileCount?: number; message?: string }) => void;

    constructor(
        context: vscode.ExtensionContext,
        outputChannel: vscode.LogOutputChannel,
    ) {
        this.context = context;
        this.outputChannel = outputChannel;
    }

    async start(): Promise<void> {
        this.serverBinaryPath = await findServerBinary(this.context);

        if (!this.serverBinaryPath) {
            const action = await vscode.window.showErrorMessage(
                'SysML language server binary not found.',
                'Install Server',
                'Set Path',
            );
            if (action === 'Set Path') {
                await vscode.commands.executeCommand(
                    'workbench.action.openSettings',
                    'sysml.server.path',
                );
            } else if (action === 'Install Server') {
                await vscode.commands.executeCommand('sysml.installServer');
            }
            throw new Error('Server binary not found');
        }

        const config = vscode.workspace.getConfiguration('sysml');
        const extraArgs: string[] = config.get<string[]>('server.extraArgs', []);

        const serverOptions: ServerOptions = {
            command: this.serverBinaryPath,
            args: extraArgs,
            transport: TransportKind.stdio,
        };

        const clientOptions: LanguageClientOptions = {
            documentSelector: [
                { language: 'sysml', scheme: 'file' },
                { language: 'sysml', scheme: 'untitled' },
                { language: 'kerml', scheme: 'file' },
                { language: 'kerml', scheme: 'untitled' },
                { language: 'toml', pattern: '**/sysml.toml', scheme: 'file' },
            ],
            synchronize: {
                fileEvents: [
                    vscode.workspace.createFileSystemWatcher(
                        '**/*.{sysml,kerml,toml}',
                    ),
                    vscode.workspace.createFileSystemWatcher(
                        '**/{sysml.lock,.project.json,.workspace.json,.meta.json}',
                    ),
                ],
            },
            outputChannel: this.outputChannel,
            traceOutputChannel: this.outputChannel,
            initializationOptions: this.getInitOptions(),
            middleware: {
                workspace: {
                    configuration: async (params, token, next) => {
                        const result = await next(params, token);
                        if (!Array.isArray(result)) {
                            return result;
                        }
                        // Translate VS Code setting keys to server keys
                        return result.map((item: Record<string, unknown>) => {
                            return this.mapConfigToServer(item);
                        });
                    },
                },
            },
        };

        this.client = new LanguageClient(
            'sysml',
            'SysML Language Server',
            serverOptions,
            clientOptions,
        );

        this.client.onDidChangeState((e) => {
            this.outputChannel.info(
                `Server state: ${stateToString(e.oldState)} -> ${stateToString(e.newState)}`,
            );
            this.stateChangeCallback?.(
                stateToString(e.oldState).toLowerCase(),
                stateToString(e.newState).toLowerCase(),
            );
        });

        // Listen for library status notifications from the server
        this.client.onNotification(
            'sysml/library/status',
            (params: unknown) => {
                if (this.libraryStatusCallback && params && typeof params === 'object') {
                    this.libraryStatusCallback(params as { phase: string; fileCount?: number; message?: string });
                }
            },
        );

        await this.client.start();
    }

    async stop(): Promise<void> {
        if (this.client) {
            this.outputChannel.info("Language server stopping");
            await this.client.stop();
            this.client = undefined;
        }
        for (const d of this.disposables) {
            d.dispose();
        }
        this.disposables.length = 0;
    }

    async restart(): Promise<void> {
        this.outputChannel.info('Restarting language server...');
        await this.stop();
        await this.start();
    }

    /**
     * Execute a command on the server via workspace/executeCommand.
     */
    async executeCommand<T>(command: string, args?: unknown[]): Promise<T> {
        if (!this.client || this.client.state !== State.Running) {
            throw new Error('Language server is not running');
        }
        return this.client.sendRequest('workspace/executeCommand', {
            command,
            arguments: args ?? [],
        });
    }

    get isRunning(): boolean {
        return this.client?.state === State.Running;
    }

    dispose(): void {
        this.stop().catch(() => {});
    }

    /**
     * F0-3: Set callback for server state changes (restart detection).
     */
    onStateChange(callback: (oldState: string, newState: string) => void): void {
        this.stateChangeCallback = callback;
    }

    /**
     * F0-6: Set callback for library status notifications.
     */
    onLibraryStatus(callback: (status: { phase: string; fileCount?: number; message?: string }) => void): void {
        this.libraryStatusCallback = callback;
    }

    private getInitOptions(): ServerInitOptions {
        const config = vscode.workspace.getConfiguration('sysml');
        return {
            resolution: config.get<boolean>('resolution.enabled', true),
            validation: config.get<boolean>('validation.enabled', true),
            resolutionTimeoutMs: config.get<number>('resolution.timeoutMs', 500),
            libraryPath: config.get<string>('library.path', ''),
            maxIndexFiles: config.get<number>('workspace.maxIndexFiles', 500),
            inlayHints: config.get<boolean>('inlayHints.enabled', true),
        };
    }

    /**
     * Maps VS Code configuration keys to the server's expected key format.
     */
    private mapConfigToServer(
        item: Record<string, unknown>,
    ): Record<string, unknown> {
        return mapConfigToServer(item);
    }
}

/**
 * Maps VS Code configuration keys to the server's expected key format.
 * Exported for unit testing.
 */
export function mapConfigToServer(
    item: Record<string, unknown>,
): Record<string, unknown> {
    const mapped: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(item)) {
        switch (key) {
            case 'sysml.resolution.enabled':
                mapped['resolution'] = value;
                break;
            case 'sysml.validation.enabled':
                mapped['validation'] = value;
                break;
            case 'sysml.resolution.timeoutMs':
                mapped['resolutionTimeoutMs'] = value;
                break;
            case 'sysml.library.path':
                mapped['libraryPath'] = value;
                break;
            case 'sysml.workspace.maxIndexFiles':
                mapped['maxIndexFiles'] = value;
                break;
            case 'sysml.inlayHints.enabled':
                mapped['inlayHints'] = value;
                break;
            case 'sysml.project.autoDetect':
                mapped['projectAutoDetect'] = value;
                break;
            default:
                mapped[key] = value;
                break;
        }
    }
    return mapped;
}

function stateToString(state: State): string {
    switch (state) {
        case State.Stopped:
            return 'Stopped';
        case State.Starting:
            return 'Starting';
        case State.Running:
            return 'Running';
        default:
            return 'Unknown';
    }
}
