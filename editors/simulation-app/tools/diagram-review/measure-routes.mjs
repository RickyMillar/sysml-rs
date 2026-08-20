/**
 * Edge-route detour diagnostic.
 *
 * Dumps every edge's routed length vs the straight-line (Manhattan) distance
 * between its endpoints for ONE view, plus the full point list. This is the
 * tool that found the root-level layer-wrapping bug: MixedExposeView's
 * Engine->Gearbox edges measured 1126px of route across a 270px gap (4.17x),
 * diving below the graph then climbing over the whole canvas.
 *
 * The G8 gate in assert-geometry.mjs now fails a tile automatically at >3x, so
 * reach for this when you need to SEE the offending path and understand why —
 * the gate tells you a route is pathological, this tells you its shape.
 *
 * Usage (needs api :8080 + vite :3010 already running in the SAME shell
 * invocation — sandbox netns is per-command, see run.sh):
 *   node tools/diagram-review/measure-routes.mjs <workspace-root> <ViewName>
 */
import { chromium } from 'playwright';
const APP='http://127.0.0.1:3010'; const ROOT=process.argv[2]; const WANT=process.argv[3];
const API='http://127.0.0.1:8080';
const cmd=async(n,p)=>{const r=await fetch(`${API}/api/command`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({command:n,params:p})}); return r.json();};
const b=await chromium.launch(); const page=await b.newPage({viewport:{width:1600,height:1000}});
await page.goto(`${APP}/run?workspace=${encodeURIComponent(ROOT)}`);
await page.waitForFunction(()=>Boolean(window.__workspaceStoreForTests),null,{timeout:30000});
let views=[]; for(let i=0;i<30;i++){const v=await cmd('sysml.query',{uri:'__workspace__',spec:{filter:{type:'view',viewpoint_id:null},projection:'summary_expand',sort:[{field:'name',dir:'asc'}],limit:1000}}); views=(v.rows??[]).filter(r=>(r.source_span?.file??'').replace(/^file:\/\//,'').startsWith(ROOT)); if(views.length)break; await page.waitForTimeout(1000);}
const v=views.find(r=>r.name===WANT); if(!v){console.log('view not found');process.exit(1);}
// Set-and-verify with retries — a single bare set races a still-settling
// workspace load, which is why this reported "no geometry" on a large model
// while shoot.mjs/assert-geometry.mjs (which both retry) worked fine.
for(let attempt=0;attempt<4;attempt++){
  await page.evaluate((id)=>{window.__workspaceStoreForTests.setSelectedViewId(null);window.__workspaceStoreForTests.setSelectedViewId(id);}, v.id);
  await page.waitForTimeout(700);
  const cur=await page.evaluate(()=>window.__workspaceStoreForTests.getSelectedViewId?.()??null);
  if(cur===v.id) break;
}
// Poll until the dump reports THIS view (a flat sleep races the async load —
// same set-and-verify the assert-geometry harness uses).
let d=null; const t0=Date.now();
while(Date.now()-t0<20000){ d=await page.evaluate(()=>window.__diagramGeometryForTests??null); if(d&&d.fit&&(!d.viewId||d.viewId===v.id)) break; if(d) console.error(`  (waiting: dump.viewId=${d.viewId} want=${v.id})`); d=null; await page.waitForTimeout(300); }
if(!d){console.log('no geometry');process.exit(1);}
const byId=new Map(d.nodes.map(n=>[n.id,n.rect]));
console.log(`## ${WANT} — ${d.nodes.length} node boxes, ${d.edges.length} edges\n`);
console.log('NODES:');
for(const n of d.nodes) console.log(`  ${n.id.slice(0,10)} ${n.container?'[container]':'          '} x=${n.rect.x|0} y=${n.rect.y|0} w=${n.rect.width|0} h=${n.rect.height|0}`);
console.log('\nEDGES (route length vs straight-line distance):');
for(const e of d.edges){
  const p=e.points; if(!p||p.length<2) {console.log(`  ${e.id.slice(0,10)} <no points>`); continue;}
  let len=0; for(let i=1;i<p.length;i++) len+=Math.abs(p[i].x-p[i-1].x)+Math.abs(p[i].y-p[i-1].y);
  const a=p[0], z=p[p.length-1];
  const direct=Math.abs(z.x-a.x)+Math.abs(z.y-a.y);
  const ratio=direct>0?(len/direct):Infinity;
  const flag=ratio>2.5?'  <<< PATHOLOGICAL':'';
  console.log(`  ${e.id.slice(0,10)} pts=${p.length} manhattanRoute=${len|0} direct=${direct|0} ratio=${ratio.toFixed(2)}x${flag}`);
  console.log(`      from (${a.x|0},${a.y|0}) to (${z.x|0},${z.y|0})`);
  console.log(`      path: ${p.map(q=>`(${q.x|0},${q.y|0})`).join(' ')}`);
}
await b.close();
