import React, { useEffect, useState } from 'react';

interface ProjectComposeEditorProps {
  projectId: string;
}

const DEFAULT_COMPOSE_TEMPLATE = `version: 1
runtime:
  db: .agentfs/compose.db
  mountpoint: ./.appfs
  backend: fuse
  init: if_missing
`;

export function ProjectComposeEditor({ projectId }: ProjectComposeEditorProps) {
  const [content, setContent] = useState<string>('');
  const [filePath, setFilePath] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(true);
  const [saving, setSaving] = useState<boolean>(false);
  const [validating, setValidating] = useState<boolean>(false);
  const [statusMessage, setStatusMessage] = useState<{ type: 'success' | 'error' | 'info'; text: string } | null>(null);
  const [isMissing, setIsMissing] = useState<boolean>(false);

  const loadCompose = async () => {
    setLoading(true);
    setStatusMessage(null);
    setIsMissing(false);
    try {
      const res = await fetch(`/api/projects/${projectId}/compose`);
      const data = await res.json();
      if (res.status === 404 && data.isMissing) {
        setIsMissing(true);
        setContent(DEFAULT_COMPOSE_TEMPLATE);
        setFilePath(data.error?.split('at ')?.[1] || '.appfs-compose.yaml');
      } else if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      } else {
        setContent(data.content || '');
        setFilePath(data.composeFilePath || '');
      }
    } catch (err: any) {
      setStatusMessage({ type: 'error', text: `Failed to load compose file: ${err.message}` });
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadCompose();
  }, [projectId]);

  const handleValidate = async () => {
    setValidating(true);
    setStatusMessage({ type: 'info', text: 'Validating against AppFS compose schema...' });
    try {
      const res = await fetch(`/api/projects/${projectId}/compose/validate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content }),
      });
      const data = await res.json();
      if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      }
      if (data.valid) {
        setStatusMessage({ type: 'success', text: '✓ Configuration is fully valid against AppFS schema.' });
      } else {
        setStatusMessage({ type: 'error', text: `Validation failed:\n${data.error}` });
      }
    } catch (err: any) {
      setStatusMessage({ type: 'error', text: `Validation request error: ${err.message}` });
    } finally {
      setValidating(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setStatusMessage({ type: 'info', text: 'Validating and saving configuration atomically...' });
    try {
      const res = await fetch(`/api/projects/${projectId}/compose`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ content }),
      });
      const data = await res.json();
      if (!res.ok) {
        throw new Error(data.error || `HTTP ${res.status}`);
      }
      setStatusMessage({ type: 'success', text: '✓ Configuration validated and saved successfully.' });
      setIsMissing(false);
      if (data.composeFilePath) {
        setFilePath(data.composeFilePath);
      }
      if (typeof data.content === 'string') {
        setContent(data.content);
      }
    } catch (err: any) {
      setStatusMessage({ type: 'error', text: `Save failed:\n${err.message}` });
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="compose-editor-panel" style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: '100%' }}>
        <span style={{ color: '#8b949e' }}>Loading AppFS compose file...</span>
      </div>
    );
  }

  return (
    <div className="compose-editor-panel">
      <div className="compose-editor-header">
        <div className="compose-editor-titles">
          <h3>AppFS Compose Configuration</h3>
          <span className="compose-file-path" title={filePath}>{filePath}</span>
        </div>
        <div className="compose-editor-actions">
          {isMissing && (
            <span className="missing-badge">File Missing (Using Template)</span>
          )}
          <button
            type="button"
            className="compose-btn secondary"
            onClick={handleValidate}
            disabled={validating || saving}
          >
            {validating ? 'Validating…' : 'Validate'}
          </button>
          <button
            type="button"
            className="compose-btn primary"
            onClick={handleSave}
            disabled={validating || saving}
          >
            {saving ? 'Saving…' : 'Validate & Save'}
          </button>
        </div>
      </div>

      {statusMessage && (
        <div className={`compose-status-alert ${statusMessage.type}`}>
          <pre style={{ margin: 0, whiteSpace: 'pre-wrap', fontFamily: 'inherit', fontSize: '12px' }}>
            {statusMessage.text}
          </pre>
        </div>
      )}

      <div className="compose-textarea-container">
        <textarea
          className="compose-textarea"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          placeholder="Enter AppFS compose YAML configuration..."
          spellCheck={false}
          disabled={saving}
        />
      </div>
    </div>
  );
}
