export function bodyTarget(catalog){
  if(!catalog?.selected||!catalog.endpoint)return null;
  const routine=catalog.routines.find(r=>r.oid===catalog.selected.oid);if(!routine)return null;
  return {host:catalog.endpoint.host,port:catalog.endpoint.port,database:catalog.database,user:catalog.current_user,schema:catalog.schema,name:routine.name,signature:routine.signature,kind:routine.kind};
}
export function sameBodyTarget(a,b){return !!a&&!!b&&['host','port','database','user','schema','name','signature','kind'].every(k=>a[k]===b[k]);}
