import { fetchAuthSession } from 'aws-amplify/auth';

const API_BASE_URL = import.meta.env.VITE_API_BASE_URL ?? '';

/**
 * Typed error for non-2xx API responses.
 * Consumers can catch this specifically:
 *   try { ... } catch (e) { if (e instanceof ApiError) { ... } }
 */
export class ApiError extends Error {
  readonly status: number;
  readonly statusText: string;
  readonly body?: string;

  constructor(status: number, statusText: string, body?: string) {
    super(`API error ${status}: ${statusText}`);
    this.name = 'ApiError';
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

/**
 * Extracts the Cognito ID token from the current Amplify Auth session.
 */
async function getIdToken(): Promise<string> {
  const session = await fetchAuthSession();
  const token = session.tokens?.idToken?.toString();
  if (!token) {
    throw new Error('Not authenticated — no ID token available');
  }
  return token;
}

/**
 * Internal fetch wrapper that attaches the Cognito ID token as a Bearer header.
 */
async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const token = await getIdToken();
  const url = `${API_BASE_URL}${path}`;

  const headers: Record<string, string> = {
    Authorization: `Bearer ${token}`,
    'Content-Type': 'application/json',
  };

  const response = await fetch(url, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });

  if (!response.ok) {
    const text = await response.text();
    throw new ApiError(response.status, response.statusText, text);
  }

  const text = await response.text();
  if (!text) {
    return {} as T; // 204 No Content or empty body
  }

  return JSON.parse(text) as T;
}

/**
 * Authenticated REST client for edgequake backend API calls.
 *
 * Usage:
 *   import { edgequakeApi } from '@/lib/api-client';
 *   const ns = await edgequakeApi.get<Namespace>('/api/v1/namespaces/default');
 */
export const edgequakeApi = {
  get<T>(path: string): Promise<T> {
    return request<T>('GET', path);
  },
  post<T>(path: string, body?: unknown): Promise<T> {
    return request<T>('POST', path, body);
  },
  put<T>(path: string, body?: unknown): Promise<T> {
    return request<T>('PUT', path, body);
  },
  delete<T>(path: string): Promise<T> {
    return request<T>('DELETE', path);
  },
};
