<script>
  import { api } from "../api.js";
  import { onMount } from "svelte";

  export let me;

  let users = [];
  let loading = true;
  let error = "";

  // Create form
  let nu = "";
  let np = "";
  let nAdmin = false;
  let creating = false;
  let createError = "";

  // Reset-password inline state
  let resetFor = null;
  let resetPw = "";

  async function load() {
    error = "";
    loading = true;
    try {
      users = await api.listUsers();
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function create() {
    createError = "";
    creating = true;
    try {
      await api.createUser(nu.trim(), np, nAdmin);
      nu = "";
      np = "";
      nAdmin = false;
      await load();
    } catch (e) {
      createError = e.message;
    } finally {
      creating = false;
    }
  }

  async function toggleAdmin(u) {
    error = "";
    try {
      await api.updateUser(u.username, { isAdmin: !u.isAdmin });
      await load();
    } catch (e) {
      error = e.message;
    }
  }

  async function remove(u) {
    if (!confirm(`Delete user "${u.username}"? This removes their TIDAL link too.`))
      return;
    error = "";
    try {
      await api.deleteUser(u.username);
      await load();
    } catch (e) {
      error = e.message;
    }
  }

  function startReset(u) {
    resetFor = u.username;
    resetPw = "";
  }

  async function doReset() {
    if (!resetPw) return;
    error = "";
    try {
      await api.updateUser(resetFor, { password: resetPw });
      resetFor = null;
      resetPw = "";
    } catch (e) {
      error = e.message;
    }
  }
</script>

<div class="stack">
  <div class="card">
    <h3>Create user</h3>
    {#if createError}<div class="error-box">{createError}</div>{/if}
    <form class="create" on:submit|preventDefault={create}>
      <input type="text" bind:value={nu} placeholder="Username" autocomplete="off" />
      <input type="password" bind:value={np} placeholder="Password" autocomplete="new-password" />
      <label class="chk">
        <input type="checkbox" bind:checked={nAdmin} /> Admin
      </label>
      <button class="btn" type="submit" disabled={creating || !nu.trim() || !np}>
        {creating ? "Creating…" : "Create"}
      </button>
    </form>
  </div>

  <div class="card">
    <h3>Users</h3>
    {#if error}<div class="error-box">{error}</div>{/if}
    {#if loading}
      <div class="center"><div class="spinner"></div></div>
    {:else}
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Username</th>
              <th>Admin</th>
              <th>TIDAL</th>
              <th class="right">Actions</th>
            </tr>
          </thead>
          <tbody>
            {#each users as u (u.username)}
              <tr>
                <td class="mono name">
                  {u.username}
                  {#if u.username === me.username}<span class="you">you</span>{/if}
                </td>
                <td>
                  {#if u.isAdmin}<span class="badge badge-ok">admin</span>{:else}<span class="muted">—</span>{/if}
                </td>
                <td>
                  {#if u.tidalLinked}<span class="badge badge-ok">✓</span>{:else}<span class="badge badge-off">no</span>{/if}
                </td>
                <td class="right actions">
                  <button class="btn-ghost btn-sm" on:click={() => startReset(u)}>Password</button>
                  <button class="btn-ghost btn-sm" on:click={() => toggleAdmin(u)}>
                    {u.isAdmin ? "Revoke admin" : "Make admin"}
                  </button>
                  {#if u.username !== me.username}
                    <button class="btn-danger btn-sm" on:click={() => remove(u)}>Delete</button>
                  {/if}
                </td>
              </tr>
              {#if resetFor === u.username}
                <tr class="reset-row">
                  <td colspan="4">
                    <div class="reset">
                      <input type="password" bind:value={resetPw} placeholder={`New password for ${u.username}`} autocomplete="new-password" />
                      <button class="btn btn-sm" on:click={doReset} disabled={!resetPw}>Set</button>
                      <button class="btn-ghost btn-sm" on:click={() => (resetFor = null)}>Cancel</button>
                    </div>
                  </td>
                </tr>
              {/if}
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </div>
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  h3 {
    font-size: 17px;
    margin-bottom: 14px;
  }
  .create {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
    align-items: center;
  }
  .create input[type="text"],
  .create input[type="password"] {
    width: auto;
    flex: 1;
    min-width: 140px;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--text);
    font-size: 14px;
    white-space: nowrap;
  }
  .chk input {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 14px;
  }
  th {
    text-align: left;
    color: var(--text-faint);
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 10px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  .right {
    text-align: right;
  }
  .name {
    white-space: nowrap;
  }
  .you {
    font-family: var(--font);
    font-size: 11px;
    color: var(--text-faint);
    background: var(--surface-2);
    border-radius: 10px;
    padding: 1px 7px;
    margin-left: 6px;
  }
  .actions {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
    flex-wrap: wrap;
  }
  .reset-row td {
    padding-top: 0;
  }
  .reset {
    display: flex;
    gap: 8px;
    align-items: center;
    background: var(--surface-2);
    padding: 10px;
    border-radius: var(--radius-sm);
  }
  .reset input {
    flex: 1;
  }
  .center {
    display: flex;
    justify-content: center;
    padding: 20px 0;
  }
</style>
