// Bounded scalar projection of causal ack ordering, NOT full vector-clock semantics.
import assert from 'node:assert/strict';
const actions = ['write', ...Array.from({length:5},(_,v)=>'ack_'+v)];
const initial = { local:0, ack:0, rejected:false };
const key = s => JSON.stringify(s);
const allowed = (s,v) => v>=s.ack && v<=s.local;
function step(s,action) {
  if(action==='write' && s.local<3) return {...s,local:s.local+1};
  if(action.startsWith('ack_')) {
    const v=Number(action.slice(4));
    return allowed(s,v) ? {...s,ack:v} : {...s,rejected:true};
  }
  return s;
}
function verify(successor) {
  const seen=new Map([[key(initial),initial]]),queue=[initial];
  for(let i=0;i<queue.length;i++){
    const s=queue[i];
    for(const action of actions){
      const n=successor(s,action);
      assert.ok(n.ack<=n.local && n.ack>=s.ack && n.local>=s.local,
        `unsafe ${action}: ${key(s)} -> ${key(n)}`);
      if(!seen.has(key(n))){
        assert.ok(seen.size<128,'state bound overflow');
        seen.set(key(n),n);queue.push(n);
      }
    }
  }
  return [...seen.values()];
}
const states=verify(step);
assert.ok(states.length>=8,'vacuous graph');
assert.ok(states.some(s=>s.local===3&&s.ack===3));
assert.ok(states.some(s=>s.rejected));
assert.throws(()=>verify((s,a)=>a==='ack_4'?{...s,ack:4}:step(s,a)),/unsafe/);
console.log(`model=causal_checkpoint_projection states=${states.length} negative_controls=1 status=PASS scope=abstract`);
