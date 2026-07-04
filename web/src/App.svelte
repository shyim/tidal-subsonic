<script>
  import { onMount } from "svelte";
  import { api } from "./api.js";
  import Login from "./lib/Login.svelte";
  import Account from "./lib/Account.svelte";
  import AdminUsers from "./lib/AdminUsers.svelte";

  let booting = true;
  let me = null; // null when signed out
  let tab = "account";

  async function refreshMe() {
    try {
      me = await api.me();
    } catch {
      me = null;
    }
  }

  onMount(async () => {
    await refreshMe();
    booting = false;
  });

  async function onAuthed() {
    await refreshMe();
    tab = "account";
  }

  async function logout() {
    try {
      await api.logout();
    } catch {}
    me = null;
  }
</script>

{#if booting}
  <div class="boot"><div class="spinner"></div></div>
{:else if !me}
  <Login on:authed={onAuthed} />
{:else}
  <div class="shell">
    <header>
      <div class="bar">
        <div class="title">
          <span class="logo">🌊</span>
          <span>TIDAL Subsonic</span>
        </div>
        <div class="right">
          <span class="who mono">{me.username}</span>
          <button class="btn-ghost btn-sm" on:click={logout}>Sign out</button>
        </div>
      </div>
      {#if me.isAdmin}
        <nav>
          <button class:active={tab === "account"} on:click={() => (tab = "account")}>
            My account
          </button>
          <button class:active={tab === "users"} on:click={() => (tab = "users")}>
            Users
          </button>
        </nav>
      {/if}
    </header>

    <main>
      {#if tab === "users" && me.isAdmin}
        <AdminUsers {me} />
      {:else}
        <Account {me} on:refresh={refreshMe} />
      {/if}
    </main>
  </div>
{/if}

<style>
  .boot {
    display: flex;
    justify-content: center;
    align-items: center;
    min-height: 100vh;
  }
  .shell {
    max-width: 720px;
    margin: 0 auto;
    padding: 0 20px 60px;
  }
  header {
    position: sticky;
    top: 0;
    background: var(--bg);
    padding-top: 20px;
    z-index: 5;
  }
  .bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-bottom: 16px;
  }
  .title {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 700;
    font-size: 18px;
  }
  .logo {
    font-size: 22px;
  }
  .right {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .who {
    color: var(--text-dim);
    font-size: 14px;
  }
  nav {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 20px;
  }
  nav button {
    background: transparent;
    color: var(--text-dim);
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 10px 14px;
    font-size: 15px;
    margin-bottom: -1px;
  }
  nav button:hover {
    color: var(--text);
  }
  nav button.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  main {
    padding-top: 4px;
  }
</style>
