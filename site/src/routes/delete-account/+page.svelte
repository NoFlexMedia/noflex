<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Doc from "$components/Doc.svelte";
  import { links } from "$lib/site";

  const toc = [
    ["steps", "Delete your account"],
    ["deleted", "What's deleted"],
    ["kept", "What's kept"],
    ["administrators", "If you're the administrator"],
    ["cannot", "If you can't sign in"],
  ];
</script>

<svelte:head>
  <title>Delete your account · NoFlex</title>
  <meta name="description" content="How to delete your NoFlex account, what's removed, and what's kept." />
</svelte:head>

<Doc eyebrow="Help" title="Delete your account" lede="Your NoFlex account lives on the server you signed in to, not with us. You can delete it yourself at any time, from the app or the web." {toc}>
  <p>This page is for the NoFlex apps by Christopher Tam, on every platform.</p>

  <h2 id="steps">Delete your account</h2>
  <h3>In the app</h3>
  <ol class="steps">
    <li>Open NoFlex, go to <strong>You</strong>, then <strong>Account</strong>.</li>
    <li>In <strong>Profile</strong>, choose <strong>Delete account</strong>.</li>
    <li>Confirm with your password, then <strong>Delete account</strong>.</li>
  </ol>
  <h3>On the web</h3>
  <ol class="steps">
    <li>Open your NoFlex server's address in a browser and sign in.</li>
    <li>Open your profile menu, then <strong>Account</strong>.</li>
    <li>At the bottom of <strong>Profile</strong>, choose <strong>Delete account</strong> and confirm with your password.</li>
  </ol>
  <p>If you sign in through another service, such as your server's single sign-on, there's no NoFlex password: you confirm by typing your username instead.</p>
  <p>You're signed out everywhere straight away, and the server finishes removing your data in the background.</p>

  <h2 id="deleted">What's deleted</h2>
  <ul>
    <li>Your account: username, display name, password, PIN and profile picture.</li>
    <li>What you've watched, your progress, ratings, favorites and playlists.</li>
    <li>Your devices, sessions and API keys, and anything prepared only for your devices.</li>
    <li>Accounts you linked, such as Trakt or single sign-on, and the tokens for them.</li>
    <li>Watch rooms you host, invites you created, and offline copies you made.</li>
    <li>Entries about you in the server's activity log.</li>
  </ul>
  <p>The app also removes that account's sign-in and downloads from your device.</p>

  <h2 id="kept">What's kept</h2>
  <ul>
    <li><strong>Backups and logs:</strong> copies in the server's database backups and log files remain until they expire under the operator's settings (a week of backups by default).</li>
    <li><strong>Linked servers:</strong> watch history your app shared with a friend's server is held by that server. Delete your account there too, or ask its operator.</li>
    <li><strong>Media files</strong> on the server are never touched. They belong to the server, not to an account.</li>
  </ul>

  <h2 id="administrators">If you're the administrator</h2>
  <p>If other administrators remain, deleting your account works as above. <strong>If you're the last administrator, deleting your account resets the whole server:</strong> every account, all watch history, libraries and settings are erased, and the server starts over at first-time setup. Your media files and the server's configuration file are kept. NoFlex warns you first, then asks you to type <strong>delete everything</strong> with your password before doing this.</p>

  <h2 id="cannot">If you can't sign in</h2>
  <p>Ask the person who runs your NoFlex server to delete your account. Administrators can delete any account from <strong>Administration → Users</strong>.</p>
  <p>We don't run NoFlex servers and have no access to the one you use, so we can't delete an account on it for you. If you're not sure who runs it, or need help, ask in <a href={links.discussions}>GitHub Discussions</a>.</p>
</Doc>

<style>
  .steps { display: grid; gap: 0.6em; padding-left: 0; counter-reset: step; }
  .steps li { position: relative; padding-left: 2.2em; counter-increment: step; }
  .steps li::before { content: counter(step); position: absolute; left: 0; top: 0.1em; display: grid; place-items: center; width: 1.5em; height: 1.5em; border-radius: 50%; background: var(--accent-soft); color: var(--accent); font-size: 0.82em; font-weight: 700; }
</style>
