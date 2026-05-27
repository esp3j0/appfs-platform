import fs from 'node:fs';
import path from 'node:path';
import { parseDocument } from 'yaml';

export const PROJECT_MOUNTPOINT_VALUE = './.appfs';

export interface ProjectComposePolicyResult {
  changed: boolean;
  composeFilePath?: string;
}

export interface ComposeMountpointContentResult {
  changed: boolean;
  content: string;
}

export function projectMountRoot(projectRoot: string): string {
  return path.join(projectRoot, '.appfs');
}

export function resolveProjectComposePath(projectRoot: string): string {
  const yamlPath = path.join(projectRoot, '.appfs-compose.yaml');
  const ymlPath = path.join(projectRoot, '.appfs-compose.yml');
  return (!fs.existsSync(yamlPath) && fs.existsSync(ymlPath)) ? ymlPath : yamlPath;
}

export function ensureProjectComposeMountpoint(projectRoot: string): ProjectComposePolicyResult {
  const composeFilePath = resolveProjectComposePath(projectRoot);
  if (!fs.existsSync(composeFilePath)) {
    return { changed: false, composeFilePath };
  }

  const original = fs.readFileSync(composeFilePath, 'utf-8');
  const doc = parseDocument(original, { keepSourceTokens: true });
  if (doc.errors.length > 0) {
    throw new Error(`Failed to parse compose file ${composeFilePath}: ${doc.errors[0].message}`);
  }

  const current = doc.getIn(['runtime', 'mountpoint']);
  if (current === PROJECT_MOUNTPOINT_VALUE) {
    return { changed: false, composeFilePath };
  }

  doc.setIn(['runtime', 'mountpoint'], PROJECT_MOUNTPOINT_VALUE);
  const next = String(doc);
  if (next !== original) {
    fs.writeFileSync(composeFilePath, next, 'utf-8');
    return { changed: true, composeFilePath };
  }

  return { changed: false, composeFilePath };
}

export function normalizeComposeMountpointContent(content: string): ComposeMountpointContentResult {
  const doc = parseDocument(content, { keepSourceTokens: true });
  if (doc.errors.length > 0) {
    return { changed: false, content };
  }

  const current = doc.getIn(['runtime', 'mountpoint']);
  if (current === PROJECT_MOUNTPOINT_VALUE) {
    return { changed: false, content };
  }

  doc.setIn(['runtime', 'mountpoint'], PROJECT_MOUNTPOINT_VALUE);
  const next = String(doc);
  return {
    changed: next !== content,
    content: next,
  };
}
