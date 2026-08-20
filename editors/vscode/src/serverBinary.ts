import * as vscode from 'vscode';
import * as path from 'path';
import * as fs from 'fs';
import { execFile } from 'child_process';
import { promisify } from 'util';
import * as https from 'https';

const execFileAsync = promisify(execFile);

const BINARY_NAME = 'sysml-lsp-server';
const GITHUB_OWNER = 'RickyMillar';
const GITHUB_REPO = 'sysml-rs';

/**
 * Find the server binary by searching multiple locations in priority order:
 * 1. User-configured path from sysml.server.path
 * 2. Bundled binary in extension's server/ directory
 * 3. Binary on the system PATH
 */
export async function findServerBinary(
    context: vscode.ExtensionContext,
): Promise<string | undefined> {
    // 1. User-configured path
    const configPath = vscode.workspace
        .getConfiguration('sysml')
        .get<string>('server.path', '');
    if (configPath && fs.existsSync(configPath)) {
        return configPath;
    }

    // 2. Bundled binary in extension directory
    const binaryName =
        process.platform === 'win32' ? `${BINARY_NAME}.exe` : BINARY_NAME;
    const bundledPath = path.join(context.extensionPath, 'server', binaryName);
    if (fs.existsSync(bundledPath)) {
        return bundledPath;
    }

    // 3. Binary on PATH
    const pathBinary = await findOnPath(binaryName);
    if (pathBinary) {
        return pathBinary;
    }

    return undefined;
}

/**
 * Search for a binary on the system PATH.
 */
async function findOnPath(binary: string): Promise<string | undefined> {
    const command = process.platform === 'win32' ? 'where' : 'which';
    try {
        const { stdout } = await execFileAsync(command, [binary]);
        const result = stdout.trim().split('\n')[0];
        if (result && fs.existsSync(result)) {
            return result;
        }
    } catch {
        // Binary not found on PATH
    }
    return undefined;
}

/**
 * Download and install the server binary from GitHub releases.
 * Returns the installed binary path on success.
 */
export async function installServer(
    context: vscode.ExtensionContext,
): Promise<string | undefined> {
    const platformSuffix = getPlatformSuffix();
    if (!platformSuffix) {
        vscode.window.showErrorMessage(
            `Unsupported platform: ${process.platform}-${process.arch}`,
        );
        return undefined;
    }

    const binaryName =
        process.platform === 'win32' ? `${BINARY_NAME}.exe` : BINARY_NAME;
    const assetName = `${BINARY_NAME}-${platformSuffix}`;

    return vscode.window.withProgress(
        {
            location: vscode.ProgressLocation.Notification,
            title: 'Installing SysML Language Server',
            cancellable: true,
        },
        async (progress, token) => {
            try {
                progress.report({ message: 'Fetching latest release...' });

                const releaseUrl = `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`;
                const releaseData = await fetchJson(releaseUrl, token);

                if (token.isCancellationRequested) {
                    return undefined;
                }

                const assets = releaseData.assets as
                    | { name: string; browser_download_url: string }[]
                    | undefined;
                const asset = assets?.find(
                    (a) => a.name === assetName,
                );
                if (!asset) {
                    vscode.window.showErrorMessage(
                        `No binary found for ${platformSuffix} in the latest release.`,
                    );
                    return undefined;
                }

                progress.report({ message: 'Downloading binary...' });

                const serverDir = path.join(context.extensionPath, 'server');
                if (!fs.existsSync(serverDir)) {
                    fs.mkdirSync(serverDir, { recursive: true });
                }

                const destPath = path.join(serverDir, binaryName);
                await downloadFile(
                    asset.browser_download_url,
                    destPath,
                    progress,
                    token,
                );

                if (token.isCancellationRequested) {
                    // Clean up partial download
                    if (fs.existsSync(destPath)) {
                        fs.unlinkSync(destPath);
                    }
                    return undefined;
                }

                // Make executable on Unix
                if (process.platform !== 'win32') {
                    fs.chmodSync(destPath, 0o755);
                }

                progress.report({ message: 'Installation complete.' });
                vscode.window.showInformationMessage(
                    'SysML Language Server installed successfully.',
                );

                return destPath;
            } catch (err) {
                const msg = err instanceof Error ? err.message : String(err);
                vscode.window.showErrorMessage(
                    `Failed to install server: ${msg}`,
                );
                return undefined;
            }
        },
    );
}

export function getPlatformSuffix(): string | undefined {
    const platform = process.platform;
    const arch = process.arch;

    // These must match the Rust target triples built by the release workflow
    // (see .github/workflows/vscode-extension.yml build-server matrix) and the
    // standalone binary asset names published in the GitHub Release.
    if (platform === 'linux' && arch === 'x64') {
        return 'x86_64-unknown-linux-gnu';
    }
    if (platform === 'linux' && arch === 'arm64') {
        return 'aarch64-unknown-linux-gnu';
    }
    if (platform === 'darwin' && arch === 'x64') {
        return 'x86_64-apple-darwin';
    }
    if (platform === 'darwin' && arch === 'arm64') {
        return 'aarch64-apple-darwin';
    }
    if (platform === 'win32' && arch === 'x64') {
        return 'x86_64-pc-windows-msvc.exe';
    }
    return undefined;
}

function fetchJson(
    url: string,
    token: vscode.CancellationToken,
): Promise<Record<string, unknown>> {
    return new Promise((resolve, reject) => {
        const parsed = new URL(url);
        const options = {
            hostname: parsed.hostname,
            path: parsed.pathname + parsed.search,
            headers: {
                'User-Agent': 'sysml-vscode',
                Accept: 'application/vnd.github.v3+json',
            },
        };

        const req = https.get(options, (res) => {
            if (
                res.statusCode &&
                res.statusCode >= 300 &&
                res.statusCode < 400 &&
                res.headers.location
            ) {
                fetchJson(res.headers.location, token).then(resolve, reject);
                return;
            }
            if (res.statusCode && res.statusCode >= 400) {
                reject(new Error(`HTTP ${res.statusCode}`));
                return;
            }

            let data = '';
            res.on('data', (chunk: Buffer) => {
                data += chunk.toString();
            });
            res.on('end', () => {
                try {
                    resolve(JSON.parse(data));
                } catch (err) {
                    reject(err);
                }
            });
        });

        req.on('error', reject);
        token.onCancellationRequested(() => req.destroy());
    });
}

function downloadFile(
    url: string,
    dest: string,
    progress: vscode.Progress<{ message?: string; increment?: number }>,
    token: vscode.CancellationToken,
): Promise<void> {
    return new Promise((resolve, reject) => {
        const file = fs.createWriteStream(dest);

        const doRequest = (requestUrl: string): void => {
            const parsed = new URL(requestUrl);
            const options = {
                hostname: parsed.hostname,
                path: parsed.pathname + parsed.search,
                headers: { 'User-Agent': 'sysml-vscode' },
            };

            const req = https.get(options, (res) => {
                if (
                    res.statusCode &&
                    res.statusCode >= 300 &&
                    res.statusCode < 400 &&
                    res.headers.location
                ) {
                    doRequest(res.headers.location);
                    return;
                }
                if (res.statusCode && res.statusCode >= 400) {
                    file.close();
                    reject(new Error(`HTTP ${res.statusCode}`));
                    return;
                }

                const totalSize = parseInt(
                    res.headers['content-length'] ?? '0',
                    10,
                );
                let downloaded = 0;

                res.on('data', (chunk: Buffer) => {
                    downloaded += chunk.length;
                    if (totalSize > 0) {
                        const pct = Math.round((downloaded / totalSize) * 100);
                        progress.report({
                            message: `Downloading... ${pct}%`,
                        });
                    }
                });

                res.pipe(file);
                file.on('finish', () => {
                    file.close();
                    resolve();
                });
            });

            req.on('error', (err) => {
                file.close();
                reject(err);
            });

            token.onCancellationRequested(() => {
                req.destroy();
                file.close();
                reject(new Error('Download cancelled'));
            });
        };

        doRequest(url);
    });
}
