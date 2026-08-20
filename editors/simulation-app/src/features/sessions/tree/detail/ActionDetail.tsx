import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import type { ActionTreeNode } from '../types';
import { DetailMeta, DetailShell } from './common';

function actionLaunch(node: ActionTreeNode): { label: string; path: string } | null {
  switch (node.rawKind) {
    case 'AnalysisCaseUsage':
    case 'AnalysisCaseDefinition':
      return { label: 'Launch analysis case', path: '/analyze/sweep' };
    case 'VerificationCaseUsage':
    case 'VerificationCaseDefinition':
      return { label: 'Launch verification case', path: '/verify' };
    default:
      return null;
  }
}

export function ActionDetail({
  node,
  testIdPrefix,
}: {
  node: ActionTreeNode;
  testIdPrefix: string;
}) {
  const navigate = useNavigate();
  const setActiveSessionTarget = useWorkspaceUIStore((s) => s.setActiveSessionTarget);
  const launch = actionLaunch(node);

  const handleLaunch = useCallback(() => {
    if (!launch) return;
    setActiveSessionTarget(node.elementId || node.id);
    navigate(launch.path);
  }, [launch, navigate, node.elementId, node.id, setActiveSessionTarget]);

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="action">
      <DetailMeta node={node} extra={node.rawKind} />
      {launch ? (
        <button
          type="button"
          data-testid={`${testIdPrefix}-action-launch`}
          onClick={handleLaunch}
          className="inline-flex items-center gap-1.5 px-2.5 py-1 transition-all"
          style={{
            alignSelf: 'flex-start',
            border: '1px solid var(--outline-variant)',
            borderRadius: 4,
            background: 'var(--primary-container)',
            color: 'var(--on-primary-container)',
            cursor: 'pointer',
            fontSize: 11,
            fontWeight: 600,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
            rocket_launch
          </span>
          {launch.label}
        </button>
      ) : (
        <div
          style={{
            fontSize: 11,
            color: 'var(--outline)',
            lineHeight: 1.5,
          }}
        >
          This action kind doesn’t have a runnable launcher yet. Live values
          aren’t surfaced for <code>{node.rawKind}</code> today.
        </div>
      )}
    </DetailShell>
  );
}
