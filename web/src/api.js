// Thin wrapper around the portal JSON API. All calls are same-origin and rely
// on the session cookie set by /api/login.

async function request(method, path, body) {
  const opts = {
    method,
    headers: {},
    credentials: "same-origin",
  };
  if (body !== undefined) {
    opts.headers["Content-Type"] = "application/json";
    opts.body = JSON.stringify(body);
  }
  const res = await fetch(path, opts);
  let data = null;
  const text = await res.text();
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      data = { error: text };
    }
  }
  if (!res.ok) {
    const msg = (data && data.error) || `Request failed (${res.status})`;
    throw new Error(msg);
  }
  return data;
}

export const api = {
  login: (username, password) =>
    request("POST", "/api/login", { username, password }),
  logout: () => request("POST", "/api/logout"),
  me: () => request("GET", "/api/me"),
  linkStart: () => request("POST", "/api/link/start"),
  linkComplete: (code, key) =>
    request("POST", "/api/link/complete", { code, key }),
  unlink: () => request("POST", "/api/unlink"),
  changePassword: (newPassword) =>
    request("POST", "/api/password", { newPassword }),
  listUsers: () => request("GET", "/api/users"),
  createUser: (username, password, isAdmin) =>
    request("POST", "/api/users", { username, password, isAdmin }),
  updateUser: (name, patch) =>
    request("POST", `/api/users/${encodeURIComponent(name)}`, patch),
  deleteUser: (name) =>
    request("DELETE", `/api/users/${encodeURIComponent(name)}`),
};
