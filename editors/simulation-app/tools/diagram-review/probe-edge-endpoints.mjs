/** For ONE view: join the scene (edge -> source/target ids) with the geometry
 *  dump (node -> rect) so a bad edge endpoint can be traced to a real node. */
import { chromium } from 'playwright';
const APP='http://127.0.0.1:3010', API='http://127.0.0.1:8080';
const ROOT=process.argv[2], WANT=process.argv[3];
const cmd=async(n,p)=>{const r=await fetch(`${API}/api/command`,{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({command:n,params:p})}); return r.json();};
const b=await chromium.launch(); const page=await b.newPage({viewport:{width:1600,height:1000}});
await page.goto(`${APP}/run?workspace=${encodeURIComponent(ROOT)}`);
await page.waitForFunction(()=>Boolean(window.__workspaceStoreForTests),null,{timeout:30000});
let views=[]; for(let i=0;i<30;i++){const v=await cmd('sysml.query',{uri:'__workspace__',spec:{filter:{type:'view',viewpoint_id:null},projection:'summary_expand',sort:[{field:'name',dir:'asc'}],limit:1000}}); views=(v.rows??[]).filter(r=>(r.source_span?.file??'').replace(/^file:\/\//,'').startsWith(ROOT)); if(views.length)break; await page.waitForTimeout(1000);}
const v=views.find(r=>r.name===WANT);
const vm=await cmd('sysml.diagram.viewmodel',{uri:'__workspace__',view_usage_id:v.id,expanded_ids:[]});
await page.evaluate((id)=>window.__workspaceStoreForTests.setSelectedViewId(id), v.id);
await page.waitForTimeout(3500);
const d=await page.evaluate(()=>window.__diagramGeometryForTests??null);
// scene edges + node/port parentage
const parent={}, names={}, ports={};
const walk=(cs,pid)=>{for(const c of cs??[]){if(c.Node){parent[c.Node.element_id]=pid;names[c.Node.element_id]=c.Node.name||c.Node.visual_kind;for(const p of c.Node.ports??[])ports[p.element_id]=c.Node.element_id;walk(c.Node.children,c.Node.element_id);}else if(c.Compartment)walk(c.Compartment.children,pid);else if(c.Island)for(const sn of c.Island.subtree?.nodes??[]){parent[sn.element_id]=pid;names[sn.element_id]=sn.name||'island-node';walk(sn.children,sn.element_id);}}};
for(const n of vm.scene.nodes??[]){parent[n.element_id]='root';names[n.element_id]=n.name||n.visual_kind;for(const p of n.ports??[])ports[p.element_id]=n.element_id;walk(n.children,n.element_id);}
const gather=(s)=>{const o=[...(s.edges??[])];const w=(c)=>{for(const x of c??[]){if(x.Edge)o.push(x.Edge);else if(x.Node)w(x.Node.children);else if(x.Compartment)w(x.Compartment.children);else if(x.Island){o.push(...(x.Island.subtree?.edges??[]));for(const sn of x.Island.subtree?.nodes??[])w(sn.children);}}};for(const n of s.nodes??[])w(n.children);return o;};
const rect=new Map((d?.nodes??[]).map(n=>[n.id,n.rect]));
const geo=new Map((d?.edges??[]).map(e=>[e.id,e.points]));
console.log(`## ${WANT}`);
for(const e of gather(vm.scene)){
  const pts=geo.get(e.id); if(!pts) continue;
  const last=pts[pts.length-1], first=pts[0];
  const off = pts.some(p=>p.y<0||p.x<0);
  if(!off) continue;
  const own=(id)=>ports[id]??id;
  const so=own(e.source_id), to=own(e.target_id);
  console.log(`\n!! edge ${e.id.slice(0,8)} label=${JSON.stringify(e.label)} kind=${JSON.stringify(e.kind).slice(0,30)}`);
  console.log(`   drawn: first=(${first.x|0},${first.y|0}) last=(${last.x|0},${last.y|0})  <-- OFF-CANVAS`);
  console.log(`   source_id=${e.source_id.slice(0,8)} port?=${e.source_id in ports} ownerNode=${so.slice(0,8)} "${names[so]}" parent=${(parent[so]??'??').slice(0,8)} rect=${JSON.stringify(rect.get(so))}`);
  console.log(`   target_id=${e.target_id.slice(0,8)} port?=${e.target_id in ports} ownerNode=${to.slice(0,8)} "${names[to]}" parent=${(parent[to]??'??').slice(0,8)} rect=${JSON.stringify(rect.get(to))}`);
  console.log(`   => same parent? ${(parent[so]??'??')===(parent[to]??'??')}`);
}
await b.close();
