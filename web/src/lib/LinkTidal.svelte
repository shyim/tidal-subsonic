<script>
  import { api } from "../api.js";
  import { createEventDispatcher } from "svelte";

  export let linked = false;

  const dispatch = createEventDispatcher();

  let phase = "idle"; // idle | pasting | working
  let linkKey = "";
  let code = "";
  let error = "";

  async function start() {
    error = "";
    phase = "working";
    try {
      const { authorizeUrl, key } = await api.linkStart();
      linkKey = key;
      window.open(authorizeUrl, "_blank", "noopener");
      phase = "pasting";
    } catch (e) {
      error = e.message;
      phase = "idle";
    }
  }

  async function complete() {
    error = "";
    phase = "working";
    try {
      await api.linkComplete(code.trim(), linkKey);
      code = "";
      dispatch("changed");
    } catch (e) {
      error = e.message;
      phase = "pasting";
    }
  }

  function cancel() {
    phase = "idle";
    code = "";
    error = "";
  }

  async function unlink() {
    if (!confirm("Unlink your TIDAL account? Streaming will stop until you re-link.")) return;
    error = "";
    phase = "working";
    try {
      await api.unlink();
      dispatch("changed");
    } catch (e) {
      error = e.message;
    } finally {
      phase = "idle";
    }
  }
</script>

<div class="link">
  <div class="head">
    <div>
      <h3>TIDAL account</h3>
      <p class="muted">Link your TIDAL login so this account can stream.</p>
    </div>
    {#if linked}
      <span class="badge badge-ok">✓ Linked</span>
    {:else}
      <span class="badge badge-off">Not linked</span>
    {/if}
  </div>

  {#if error}<div class="error-box">{error}</div>{/if}

  {#if phase === "pasting"}
    <div class="paste">
      <p class="muted step">
        A TIDAL login opened in a new tab. After you authorize, you'll land on a
        page whose URL contains <code class="mono">code=…</code> — paste that URL
        (or just the code) here.
      </p>
      <div class="field">
        <input
          type="text"
          bind:value={code}
          placeholder="Paste the redirect URL or code"
        />
      </div>
      <div class="actions">
        <button class="btn" on:click={complete} disabled={!code.trim()}>
          Complete linking
        </button>
        <button class="btn-ghost" on:click={cancel}>Cancel</button>
      </div>
    </div>
  {:else if linked}
    <div class="actions">
      <button class="btn-ghost" on:click={start} disabled={phase === "working"}>
        Re-link
      </button>
      <button class="btn-danger" on:click={unlink} disabled={phase === "working"}>
        Unlink
      </button>
    </div>
  {:else}
    <button class="btn" on:click={start} disabled={phase === "working"}>
      {phase === "working" ? "Opening TIDAL…" : "Link TIDAL account"}
    </button>
  {/if}
</div>

<style>
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 14px;
  }
  .head h3 {
    font-size: 17px;
    margin-bottom: 3px;
  }
  .head .muted {
    margin: 0;
  }
  .actions {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }
  .step {
    margin-top: 0;
  }
  .badge {
    flex-shrink: 0;
    white-space: nowrap;
  }
</style>
