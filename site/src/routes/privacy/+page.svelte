<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Doc from "$components/Doc.svelte";
  import { links } from "$lib/site";

  const toc = [
    ["summary", "The short version"],
    ["scope", "What this covers"],
    ["us", "What we collect"],
    ["server", "Your NoFlex server"],
    ["device", "On your device"],
    ["third-parties", "Services your server may contact"],
    ["platforms", "App stores and platforms"],
    ["permissions", "App permissions"],
    ["deletion", "Deleting your data"],
    ["children", "Children"],
    ["rights", "Your rights"],
    ["changes", "Changes"],
    ["contact", "Contact"],
  ];
</script>

<svelte:head>
  <title>Privacy policy · NoFlex</title>
  <meta name="description" content="NoFlex is software you run yourself. The apps send nothing to us, and this page explains where your data lives and who can see it." />
</svelte:head>

<Doc eyebrow="Legal" title="Privacy policy" lede="NoFlex is software you run yourself. Your library, your account and what you watch live on a server you or someone you know controls, never with us." updated="11 November 2026" {toc}>
  <h2 id="summary">The short version</h2>
  <ul>
    <li><strong>We don't collect anything through the apps.</strong> No analytics, no crash reporting, no advertising, no tracking, and no account with us.</li>
    <li><strong>Your data lives on your NoFlex server.</strong> Whoever runs that server (its <em>operator</em>) holds your account and watch history and is responsible for them. If you run your own server, that's you.</li>
    <li><strong>The apps talk only to the servers you connect them to.</strong> Media never passes through us.</li>
    <li><strong>By default, a server contacts three outside services</strong> to identify what's in its library: TMDB, TheTVDB and MusicBrainz. Others are only contacted if the operator sets them up.</li>
  </ul>

  <h2 id="scope">What this covers</h2>
  <p>This policy covers the NoFlex apps for phones, tablets, computers and TVs, the web client built into the server, the NoFlex server software, and this website, noflex.dev. NoFlex is made by Christopher Tam (“we”, “us”), who maintains the project.</p>
  <p>Because NoFlex is self-hosted, there are two parties in it: us, who make the software, and the operator of each server, who runs it. We have no access to anyone's server and can't see, change or delete what's on it.</p>

  <h2 id="us">What we collect</h2>
  <h3>Through the apps and the server</h3>
  <p>Nothing. The apps and the server contain no analytics, telemetry, crash reporting, advertising or tracking code, and don't check in with us for updates or anything else.</p>
  <h3>On this website</h3>
  <p>This site sets no cookies and runs no analytics. It's hosted by Cloudflare, which, like any web host, processes your IP address and browser details to deliver pages and protect the site from abuse, under <a href="https://www.cloudflare.com/privacypolicy/">Cloudflare's privacy policy</a>. We don't use that data to identify you.</p>
  <h3>When you contact us</h3>
  <p>We're reached through <a href={links.repo}>GitHub</a>. Issues, discussions and pull requests there are public, and governed by <a href="https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement">GitHub's privacy statement</a>. You can edit or delete what you post.</p>

  <h2 id="server">Your NoFlex server</h2>
  <p>To work, a server keeps the following about each person with an account. It's stored in the server's database and is visible to its administrators.</p>
  <table>
    <thead><tr><th>What</th><th>Why</th></tr></thead>
    <tbody>
      <tr><td>Username, display name, role, and an optional profile picture</td><td>To sign you in and show who's watching. Pictures are re-encoded so only the pixels are kept.</td></tr>
      <tr><td>Your password and PIN, as one-way hashes</td><td>To check them. Nobody, administrators included, can read them back.</td></tr>
      <tr><td>Watch history, positions, ratings, favorites and playlists</td><td>To resume where you left off, keep track of what you've seen, and recommend what to watch next.</td></tr>
      <tr><td>Your devices and sessions: device name, app and version, what the device can play, IP address, and when it was last used</td><td>To keep you signed in, choose a version each device can play, and let you see and sign out your devices.</td></tr>
      <tr><td>An activity log of sign-ins and administrative actions, which can include IP addresses</td><td>So administrators can see what happened on their server.</td></tr>
      <tr><td>If you use them: the email of an account you sign in with through your operator's single sign-on, tokens for services you link (such as Trakt), and keys for friends' servers you link</td><td>To sign you in that way, and to act for you with those services.</td></tr>
    </tbody>
  </table>
  <p>Operators also keep database backups and server logs, which hold copies of this data for as long as their settings say (a week of backups by default).</p>

  <h2 id="device">On your device</h2>
  <ul>
    <li><strong>Apps for phones, tablets and computers</strong> keep your sign-in tokens in the system's secure storage (Keychain on Apple devices), plus the server's address, a random identifier for the device, your settings, anything you download for offline viewing, and a cache of artwork. Downloads are excluded from iCloud backups.</li>
    <li><strong>The web client</strong> keeps your sign-in for the browser tab only (session storage), and remembers a random device identifier and a few preferences such as volume. It sets no cookies.</li>
    <li><strong>The TV app</strong> keeps your sign-in, the server's address and your settings in the TV's app storage.</li>
  </ul>
  <p>The apps send your device's name (for example, the model of your phone or your computer's name) to your server, so you can tell your devices apart in your account, where you can also sign any of them out.</p>

  <h2 id="third-parties">Services your server may contact</h2>
  <h3>On by default</h3>
  <p>To identify titles and fetch artwork, a server sends what it read from your file and folder names (titles, years, episode numbers) and the IDs those services use, along with the server's IP address, to:</p>
  <ul>
    <li><a href="https://www.themoviedb.org/privacy-policy">The Movie Database (TMDB)</a></li>
    <li><a href="https://thetvdb.com/privacy-policy">TheTVDB</a></li>
    <li><a href="https://metabrainz.org/privacy">MusicBrainz</a></li>
  </ul>
  <p>No account information, watch history or media is sent to them. Your apps load artwork from your server, not from these services.</p>
  <h3>Only if the operator sets them up</h3>
  <ul>
    <li><strong>Single sign-on</strong> (OpenID Connect): your sign-in goes through the provider your operator chose.</li>
    <li><strong>Trakt:</strong> if you link your Trakt account, what you watch and how far you got is sent to Trakt.</li>
    <li><strong>Webhooks:</strong> events such as what's playing, including usernames and titles, go to the address the operator configured.</li>
    <li><strong>A remote embedding service:</strong> NoFlex understands searches with a model that runs on the server itself. If an operator points it at an outside service instead, synopses and the searches people type are sent there.</li>
    <li><strong>Live TV:</strong> channel lists, programme guides and channel logos are fetched from the sources the operator adds.</li>
    <li><strong>Google Cast:</strong> if the operator turns it on, the web client loads Google's Cast software from Google when you use it in Chrome, and a Cast device you play to receives the stream's address, the title and its artwork. <a href="https://policies.google.com/privacy">Google's privacy policy</a> applies.</li>
  </ul>
  <h3>Friends' servers</h3>
  <p>If you link a friend's server, your app connects to it directly and signs in with a key it issued you. To keep what you've watched in step between them, your app shares your watch history for titles both servers have (which title, whether you finished it, and how far you got) with that server. You can turn this off for each linked server. That server's operator then holds that data under their own practices.</p>

  <h2 id="platforms">App stores and platforms</h2>
  <p>When you download NoFlex from an app store, the store (such as Apple, Google, Amazon, Samsung, LG or Microsoft) handles the download under its own privacy policy. Stores may share aggregate statistics with us, such as download counts, and crash reports if you've agreed to share them with developers in your device's settings. Your device's operating system may also have its own services, such as the system web view on Android TV, that work under the platform's policies.</p>

  <h2 id="permissions">App permissions</h2>
  <ul>
    <li><strong>Network and local network:</strong> to reach your server, which is often on your home network. On iPhone, iPad and Mac, the system asks before the app can see devices on your local network.</li>
    <li><strong>Notifications</strong> (Android): to show the progress of downloads. Nothing is pushed from a server.</li>
    <li><strong>Background activity:</strong> to finish downloads and keep Smart Downloads up to date while the app isn't open.</li>
    <li><strong>Picture-in-picture:</strong> to keep playing in a small window.</li>
    <li><strong>Photos you pick:</strong> only the picture you choose as your profile picture, which is uploaded to your server.</li>
  </ul>

  <h2 id="deletion">Deleting your data</h2>
  <ul>
    <li><strong>Your account:</strong> delete it yourself from your account settings in the app or the web client. <a href="/delete-account">How to delete your account</a> has the steps. This removes your account and everything tied to it from the server, though the operator's backups and logs keep copies until they expire.</li>
    <li><strong>On your device:</strong> signing out removes that account's sign-in and downloads from the app. Uninstalling the app removes everything it stored.</li>
  </ul>

  <h2 id="children">Children</h2>
  <p>NoFlex isn't directed at children, and we collect no data from anyone, children included. Operators can give younger viewers their own accounts with parental controls: allowed ratings, libraries, and a PIN on the profile.</p>

  <h2 id="rights">Your rights</h2>
  <p>Depending on where you live, laws such as the GDPR or the CCPA give you rights to access, correct, delete or move your personal data. For data on a NoFlex server, the server's operator is the one who can act on them, and NoFlex gives them the tools to: you can see your own sessions and devices, change your details, and delete your account yourself. We hold no personal data from the apps or servers to act on, and we don't sell or share personal data.</p>

  <h2 id="changes">Changes</h2>
  <p>We'll post any change to this policy on this page with a new date, and note significant changes in the release notes.</p>

  <h2 id="contact">Contact</h2>
  <p>Christopher Tam maintains NoFlex. Ask about this policy in <a href={links.discussions}>GitHub Discussions</a>, and report a security issue privately through <a href={links.advisories}>GitHub's security advisories</a>.</p>
</Doc>
