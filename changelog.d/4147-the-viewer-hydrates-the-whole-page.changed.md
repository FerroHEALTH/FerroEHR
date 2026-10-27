- The viewer hydrates the whole page on FerroUI 0.5: the server draws the
  first page and the browser draws every page after it, so a link or a
  filter no longer reloads the document, and the browser's back and forward
  walk the pages it drew. The pages load their code in six groups as they
  are first opened. Every page still works with no WebAssembly, and a post
  still answers with the page it was posted to.
