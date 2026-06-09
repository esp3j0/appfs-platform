import type {
  CreatePrincipalRequest,
  PrincipalListResponse,
  PrincipalResumeRequest,
  PrincipalStartRequest,
} from './types';

export async function listProjectPrincipals(projectId: string): Promise<PrincipalListResponse> {
  return requestJson<PrincipalListResponse>(`/api/projects/${encodeURIComponent(projectId)}/principals`);
}

export async function createProjectPrincipal(
  projectId: string,
  body: CreatePrincipalRequest,
): Promise<unknown> {
  return requestJson(`/api/projects/${encodeURIComponent(projectId)}/principals`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export async function deleteProjectPrincipal(projectId: string, principalId: string): Promise<unknown> {
  return requestJson(
    `/api/projects/${encodeURIComponent(projectId)}/principals/${encodeURIComponent(principalId)}`,
    { method: 'DELETE' },
  );
}

export async function startProjectPrincipal(
  projectId: string,
  principalId: string,
  body: PrincipalStartRequest,
): Promise<unknown> {
  return requestJson(
    `/api/projects/${encodeURIComponent(projectId)}/principals/${encodeURIComponent(principalId)}/start`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    },
  );
}

export async function stopProjectPrincipal(projectId: string, principalId: string): Promise<unknown> {
  return requestJson(
    `/api/projects/${encodeURIComponent(projectId)}/principals/${encodeURIComponent(principalId)}/stop`,
    { method: 'POST' },
  );
}

export async function resumeProjectPrincipal(
  projectId: string,
  principalId: string,
  body: PrincipalResumeRequest = {},
): Promise<unknown> {
  return requestJson(
    `/api/projects/${encodeURIComponent(projectId)}/principals/${encodeURIComponent(principalId)}/resume`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    },
  );
}

async function requestJson<T = unknown>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(errorMessage(data) || `HTTP ${res.status}`);
  }
  return data as T;
}

function errorMessage(data: unknown): string | null {
  if (data && typeof data === 'object' && 'error' in data) {
    const value = (data as { error?: unknown }).error;
    return typeof value === 'string' ? value : null;
  }
  return null;
}
