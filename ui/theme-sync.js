// Version checks never reload subscription/list/article data. One request at a time.
(function(global) {
  'use strict';
  function createSync({read, revision, apply, active, onError=()=>{}}) {
    let running=null, again=false, disposed=false;
    async function check(force=false) {
      if (disposed || (!force && !active())) return;
      if (running) { again ||= force; return running; }
      running=(async()=>{
        try { const next=await read(revision()); if (!disposed && next) apply(next); }
        catch(error) { if(!disposed) onError(error); }
      })();
      await running;running=null;
      if(again && !disposed) { again=false;return check(true); }
    }
    return {check,dispose(){disposed=true;again=false;}};
  }
  global.RustRssThemeSync={createSync};
})(typeof window==='undefined'?globalThis:window);
