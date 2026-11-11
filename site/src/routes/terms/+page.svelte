<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Doc from "$components/Doc.svelte";
  import { links } from "$lib/site";

  const toc = [
    ["software", "The software"],
    ["content", "Your content"],
    ["servers", "Running a server"],
    ["providers", "Metadata providers"],
    ["stores", "App stores"],
    ["warranty", "No warranty"],
    ["marks", "Name and logo"],
    ["changes", "Changes"],
  ];
</script>

<svelte:head>
  <title>Terms of use · NoFlex</title>
  <meta name="description" content="The terms for using NoFlex's apps, server software and website." />
</svelte:head>

<Doc eyebrow="Legal" title="Terms of use" lede="NoFlex is free software. These terms cover using the apps, the server and this website, alongside the license the code comes under." updated="11 November 2026" {toc}>
  <h2 id="software">The software</h2>
  <p>NoFlex's source code is licensed under the <a href={links.license}>GNU Affero General Public License, version 3</a>. That license, not these terms, says what you may do with the code: run, study, change and share it, as long as people who use a changed version you run can get its source. Plugins may use any license; <a href={links.pluginLicensing}>Plugin licensing</a> explains how.</p>
  <p>NoFlex is made by Christopher Tam (“we”, “us”), who maintains the project and holds its copyright, with contributions made under the project's <a href={links.cla}>contributor license agreement</a>.</p>

  <h2 id="content">Your content</h2>
  <p>NoFlex doesn't come with or provide any films, shows, music or other media. It plays what you add to your own server. You're responsible for having the right to the media you add, stream and share, and for complying with the law where you live. Don't use NoFlex to infringe anyone's copyright.</p>

  <h2 id="servers">Running a server</h2>
  <p>If you run a NoFlex server for other people, you're responsible for it: who you invite, what you share with them, how you secure it, and the personal data it keeps about them. Our <a href="/privacy">privacy policy</a> describes what the server stores, to help you tell your users.</p>

  <h2 id="providers">Metadata providers</h2>
  <p>A server identifies titles using TMDB, TheTVDB and MusicBrainz, whose own terms apply to their data and artwork. NoFlex ships shared keys for TMDB and TheTVDB for personal, non-commercial use. If you use NoFlex commercially, use your own keys under those providers' terms.</p>

  <h2 id="stores">App stores</h2>
  <p>If you get a NoFlex app from an app store, that store's terms also apply to how you download and use it. For the App Store, Apple's <a href="https://www.apple.com/legal/internet-services/itunes/dev/stdeula/">standard licensed application end user license agreement</a> applies, and the source of every app stays available to you under the AGPL.</p>

  <h2 id="warranty">No warranty</h2>
  <p>NoFlex is pre-release software, made available “as is”, without warranties of any kind, to the extent the law allows. We aren't liable for any loss arising from its use, including lost data, again to the extent the law allows. Keep backups of what matters to you. Sections 15 and 16 of the AGPL say the same for the code.</p>

  <h2 id="marks">Name and logo</h2>
  <p>The NoFlex name and logo identify the official project. You're welcome to use them to refer to NoFlex. If you distribute a changed version, give it a name and logo that make clear it isn't the official one. Other names and logos on this site belong to their owners.</p>

  <h2 id="changes">Changes</h2>
  <p>We'll post changes to these terms here with a new date. Ask about them in <a href={links.discussions}>GitHub Discussions</a>.</p>
</Doc>
