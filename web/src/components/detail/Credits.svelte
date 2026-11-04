<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<script>
  /** `credits`: one entry per part, in billing order. */
  let { credits = [] } = $props();

  const CREW_ORDER = [["Director", "Directed by"], ["Writer", "Written by"], ["Producer", "Produced by"], ["Composer", "Music by"]];
  let crew = $derived(CREW_ORDER.map(([role, label]) => ({ label, people: credits.filter((credit) => credit.role === role) })).filter((group) => group.people.length));
  // Providers don't always name an actor's role, and anything else that isn't
  // crew above still belongs on the page.
  let cast = $derived(credits.filter((credit) => !CREW_ORDER.some(([role]) => role === credit.role)));

  const initials = (name) => name.split(/\s+/).filter(Boolean).slice(0, 2).map((part) => part[0]).join("").toUpperCase();
  const hue = (name) => [...name].reduce((sum, char) => sum + char.charCodeAt(0), 0) % 360;
  const searchFor = (name) => `/search?q=${encodeURIComponent(name)}`;
</script>

{#if credits.length}
  <section class="section credits">
    <div class="section-head"><h2>Cast &amp; Crew</h2></div>
    {#if crew.length}
      <dl class="crew">
        {#each crew as group (group.label)}
          <div>
            <dt>{group.label}</dt>
            <dd>{#each group.people as person, index (person.personId)}{index ? ", " : ""}<a href={searchFor(person.name)}>{person.name}</a>{/each}</dd>
          </div>
        {/each}
      </dl>
    {/if}
    {#if cast.length}
      <ul class="cast">
        {#each cast as person (person.personId + (person.character ?? ""))}
          <li>
            <a href={searchFor(person.name)}>
              <span class="face" style:--h={hue(person.name)} aria-hidden="true">{initials(person.name)}</span>
              <strong>{person.name}</strong>
              {#if person.character || person.role}<small>{person.character ?? person.role}</small>{/if}
            </a>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .credits { margin-top: 56px; }
  .crew { display: flex; flex-wrap: wrap; gap: 14px 40px; margin-bottom: 22px; }
  dt { font-size: 0.76rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-3); margin-bottom: 4px; }
  dd { font-size: 0.94rem; font-weight: 550; }
  dd a:hover { text-decoration: underline; text-underline-offset: 3px; text-decoration-color: var(--text-3); }
  .cast { display: grid; grid-auto-flow: column; grid-auto-columns: 112px; gap: 14px; overflow-x: auto; padding: 4px 2px 10px; scroll-snap-type: x proximity; scrollbar-width: none; list-style: none; margin: 0; }
  .cast li { scroll-snap-align: start; min-width: 0; }
  .cast a { display: grid; justify-items: center; gap: 2px; text-align: center; }
  .face { display: grid; place-items: center; width: 84px; height: 84px; margin-bottom: 8px; border-radius: 50%; background: hsl(var(--h) 24% 18%); color: hsl(var(--h) 40% 82%); font-size: 1.3rem; font-weight: 650; letter-spacing: -0.02em; box-shadow: 0 0 0 1px rgb(255 255 255 / 0.07); transition: transform var(--t) var(--ease-out), box-shadow var(--t); }
  .cast a:hover .face { transform: translateY(-2px); box-shadow: 0 0 0 1px rgb(255 255 255 / 0.2); }
  strong { font-size: 0.84rem; font-weight: 600; line-height: 1.25; max-width: 100%; }
  small { color: var(--text-3); font-size: 0.78rem; line-height: 1.25; max-width: 100%; }
</style>
