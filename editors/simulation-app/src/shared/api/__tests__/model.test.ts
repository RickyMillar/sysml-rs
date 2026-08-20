import { describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ httpPost: vi.fn() }));

vi.mock('../http', () => ({
  httpGet: vi.fn(),
  httpPost: mocks.httpPost,
}));

import { fileUriToLocalPath, loadFile } from '../model';

describe('fileUriToLocalPath', () => {
  it('converts a file URI from workspace/tree APIs to the /files path shape', () => {
    expect(fileUriToLocalPath('file:///workspace/Production%20Cell.sysml')).toBe(
      '/workspace/Production Cell.sysml',
    );
  });

  it('leaves ordinary paths and non-local file authorities untouched', () => {
    expect(fileUriToLocalPath('/workspace/Production Cell.sysml')).toBe(
      '/workspace/Production Cell.sysml',
    );
    expect(fileUriToLocalPath('file://remote-host/share/Model.sysml')).toBe(
      'file://remote-host/share/Model.sysml',
    );
  });

  it('removes the URI-only leading slash for a Windows drive path', () => {
    expect(fileUriToLocalPath('file:///C:/models/Model.sysml')).toBe('C:/models/Model.sysml');
  });
});

describe('loadFile', () => {
  it('passes a filesystem path, not a file URI, to the REST /files endpoint', async () => {
    mocks.httpPost.mockResolvedValue({
      uri: '/workspace/Model.sysml',
      source: 'package Model {}',
    });

    await loadFile('file:///workspace/Model.sysml');

    expect(mocks.httpPost).toHaveBeenCalledWith('/files', { path: '/workspace/Model.sysml' });
  });
});
