- The viewer takes FerroUI 0.5.4: its skip link keeps the browser's own
  fragment navigation under hydration, so the viewer no longer patches the
  link after the page hydrates, and a code editor's key costs the edit, not
  the length of the text.
