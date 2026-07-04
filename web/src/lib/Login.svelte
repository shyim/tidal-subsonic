<script>
  import { api } from "../api.js";
  import { createEventDispatcher } from "svelte";

  const dispatch = createEventDispatcher();

  let username = "";
  let password = "";
  let error = "";
  let loading = false;

  async function submit() {
    error = "";
    loading = true;
    try {
      await api.login(username.trim(), password);
      dispatch("authed");
    } catch (e) {
      error = e.message;
    } finally {
      loading = false;
    }
  }
</script>

<div class="wrap">
  <div class="brand">
    <span class="logo">🌊</span>
    <h1>TIDAL Subsonic</h1>
    <p class="muted">Sign in to manage your account</p>
  </div>

  <form class="card" on:submit|preventDefault={submit}>
    {#if error}<div class="error-box">{error}</div>{/if}

    <div class="field">
      <label for="u">Username</label>
      <input
        id="u"
        type="text"
        bind:value={username}
        autocomplete="username"
        placeholder="Your Subsonic username"
      />
    </div>

    <div class="field">
      <label for="p">Password</label>
      <input
        id="p"
        type="password"
        bind:value={password}
        autocomplete="current-password"
        placeholder="Your Subsonic password"
      />
    </div>

    <button
      class="btn btn-block"
      type="submit"
      disabled={loading || !username || !password}
    >
      {loading ? "Signing in…" : "Sign in"}
    </button>
  </form>

  <p class="muted foot">
    Accounts are created by an admin. Once signed in, link your own TIDAL
    account.
  </p>
</div>

<style>
  .wrap {
    max-width: 400px;
    margin: 0 auto;
    padding: 8vh 20px 40px;
  }
  .brand {
    text-align: center;
    margin-bottom: 24px;
  }
  .logo {
    font-size: 44px;
    display: block;
    margin-bottom: 8px;
  }
  .brand h1 {
    font-size: 26px;
    margin-bottom: 4px;
  }
  .foot {
    text-align: center;
    margin-top: 20px;
  }
</style>
