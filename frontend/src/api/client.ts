export async function apiFetch(path: string, opts?: RequestInit): Promise<Response> {
  const apiKey = localStorage.getItem('api_key') ?? '';
  return fetch(path, {
    ...opts,
    headers: {
      ...opts?.headers,
      'api_key': apiKey,
    },
  });
}
