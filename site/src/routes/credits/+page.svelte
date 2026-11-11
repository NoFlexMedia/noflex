<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  import Doc from "$components/Doc.svelte";
  import { links, poster } from "$lib/site";

  const by3 = { label: "CC BY 3.0", href: "https://creativecommons.org/licenses/by/3.0/" };
  const by4 = { label: "CC BY 4.0", href: "https://creativecommons.org/licenses/by/4.0/" };
  // Attribution lines as each poster's rights holder gives them on Wikimedia Commons.
  const films = [
    { slug: "sintel", title: "Sintel", year: 2010, credit: "© copyright Blender Foundation | durian.blender.org", license: by3 },
    { slug: "big-buck-bunny", title: "Big Buck Bunny", year: 2008, credit: "© copyright Blender Foundation | www.bigbuckbunny.org", license: by3 },
    { slug: "tears-of-steel", title: "Tears of Steel", year: 2012, credit: "(CC) Blender Foundation | Project Mango", license: by3 },
    { slug: "elephants-dream", title: "Elephants Dream", year: 2006, credit: "Bassam Kurdali, Andy Goralczyk, and the Blender Foundation", license: by4 },
    { slug: "caminandes", title: "Caminandes", year: 2013, credit: "Blender Foundation, caminandes.com", license: by3 },
    { slug: "sprite-fright", title: "Sprite Fright", year: 2021, credit: "Blender Studio", license: by4 },
    { slug: "charge", title: "Charge", year: 2022, credit: "Blender Studio", license: by4 },
  ];
  const toc = [["artwork", "Artwork"], ["metadata", "Metadata"], ["type-and-icons", "Type and icons"], ["software", "Software"], ["trademarks", "Trademarks"]];
</script>

<svelte:head>
  <title>Credits and licenses · NoFlex</title>
  <meta name="description" content="The films, data, typefaces, icons and software NoFlex and this site are built with, and their licenses." />
</svelte:head>

<Doc eyebrow="Legal" title="Credits and licenses" lede="NoFlex stands on a lot of generous work. Here's whose, and under what terms." {toc}>
  <h2 id="artwork">Artwork</h2>
  <p>The library on this site and in the screenshots is <a href={links.blender}>Blender Studio's open movies</a>, made with Blender and released under Creative Commons. The posters and stills are scaled and cropped. Tears of Steel's footage may not be used to advertise, so only its poster appears here.</p>
  <ul class="films">
    {#each films as film}
      <li>
        <img src={poster(film.slug)} alt="" width="480" height="720" loading="lazy" decoding="async" />
        <span><strong>{film.title}</strong> ({film.year})<small>{film.credit} · <a href={film.license.href}>{film.license.label}</a></small></span>
      </li>
    {/each}
  </ul>

  <h2 id="metadata">Metadata</h2>
  <ul>
    <li>This product uses TMDB and the TMDB APIs but is not endorsed, certified, or otherwise approved by <a href="https://www.themoviedb.org/">TMDB</a>.</li>
    <li>Metadata provided by <a href="https://thetvdb.com/">TheTVDB</a>. Please consider adding missing information or <a href="https://thetvdb.com/subscribe">subscribing</a>.</li>
    <li>Music metadata is powered with data from <a href="https://musicbrainz.org/">MusicBrainz</a>.</li>
  </ul>

  <h2 id="type-and-icons">Type and icons</h2>
  <ul>
    <li><a href="https://rsms.me/inter/">Inter</a> by Rasmus Andersson, under the SIL Open Font License 1.1.</li>
    <li><a href="https://phosphoricons.com/">Phosphor Icons</a>, under the MIT License.</li>
    <li><a href="https://simpleicons.org/">Simple Icons</a>, under CC0. The marks themselves are trademarks of their owners and don't imply endorsement.</li>
  </ul>

  <h2 id="software">Software</h2>
  <p>This site is built with <a href="https://svelte.dev/">SvelteKit</a>, under the MIT License. The apps and the server list every third-party component they ship, with its license, under About in each app and in the notices that come with each release. The <a href={links.repo}>source repository</a> has the whole picture.</p>

  <h2 id="trademarks">Trademarks</h2>
  <p>Plex is a trademark of Plex, Inc. NoFlex reads folders laid out the way Plex names them, and isn't affiliated with or endorsed by Plex. Other product names and marks on this site belong to their owners and are used only to say what NoFlex works with.</p>
</Doc>

<style>
  .films { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr)); gap: 10px; padding-left: 0; }
  :global(.prose) .films > li::before { display: none; }
  .films li { display: grid; grid-template-columns: 48px minmax(0, 1fr); gap: 14px; align-items: center; padding: 10px; border-radius: var(--r); background: var(--surface); border: 1px solid var(--line); }
  .films img { width: 48px; height: auto; aspect-ratio: 2 / 3; object-fit: cover; border-radius: 6px; }
  .films span { display: grid; line-height: 1.45; font-size: 0.92rem; }
  .films strong { color: var(--text); }
  .films small { color: var(--text-3); font-size: 0.8rem; }
</style>
