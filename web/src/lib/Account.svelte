<script>
  import { api } from "../api.js";
  import CopyField from "./CopyField.svelte";
  import LinkTidal from "./LinkTidal.svelte";
  import { createEventDispatcher } from "svelte";

  export let me;

  const dispatch = createEventDispatcher();

  let newPassword = "";
  let pwError = "";
  let pwOk = false;
  let pwLoading = false;

  async function savePassword() {
    pwError = "";
    pwOk = false;
    pwLoading = true;
    try {
      await api.changePassword(newPassword);
      newPassword = "";
      pwOk = true;
      setTimeout(() => (pwOk = false), 2500);
    } catch (e) {
      pwError = e.message;
    } finally {
      pwLoading = false;
    }
  }
</script>

<div class="stack">
  <div class="card">
    <LinkTidal linked={me.tidalLinked} on:changed={() => dispatch("refresh")} />
  </div>

  <div class="card">
    <h3>Connect a Subsonic client</h3>
    <p class="muted spc">
      Use these in any Subsonic app. The password is the one you sign in with here.
    </p>
    <CopyField label="Server URL" value={me.serverUrl} />
    <CopyField label="Username" value={me.username} />
  </div>

  <div class="card">
    <h3>Change password</h3>
    <p class="muted spc">Updates both this portal and your Subsonic login.</p>
    {#if pwError}<div class="error-box">{pwError}</div>{/if}
    <form on:submit|preventDefault={savePassword}>
      <div class="field">
        <label for="np">New password</label>
        <input
          id="np"
          type="password"
          bind:value={newPassword}
          autocomplete="new-password"
          placeholder="New password"
        />
      </div>
      <button class="btn" type="submit" disabled={pwLoading || !newPassword}>
        {pwLoading ? "Saving…" : pwOk ? "Saved ✓" : "Update password"}
      </button>
    </form>
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
    margin-bottom: 3px;
  }
  .spc {
    margin-top: 0;
    margin-bottom: 16px;
  }
</style>
