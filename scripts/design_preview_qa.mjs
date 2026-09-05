// 本地 HTML 样稿验收；浏览器操作经 web-access CDP Proxy，仅使用指定的预览 tab。
import fs from 'node:fs';
import vm from 'node:vm';
const target = process.argv[2];
if (!target) throw new Error('Usage: node scripts/design_preview_qa.mjs PREVIEW_TARGET_ID');
for (const name of ['index', 'a', 'b', 'c']) {
  const html = fs.readFileSync(`docs/design-preview/${name}.html`, 'utf8');
  for (const match of html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)) new vm.Script(match[1], { filename: `${name}.html` });
  if (/(?:src|href)=["']https?:\/\//i.test(html) || /\b(?:fetch|XMLHttpRequest|WebSocket|EventSource)\s*\(/.test(html)) throw new Error(`${name}: unexpected network dependency`);
  console.log(`PASS ${name}: JavaScript 语法及无外部依赖`);
}
const config = {
  a: { sessionSearch: '#sessionSearch', sessions: '#sessionCards [data-session]', workspace: '#workspace', search: '#promptSearch', turns: '#turns [data-turn]', reader: '#reader', final: '#finalAnswer', jump: '#finalButton', back: '#back', close: '#closeHelp' },
  b: { sessionSearch: '#session-search', sessions: '#sessions .session-row', workspace: '#reader', search: '#prompt-search', turns: '#turns .turn-btn', reader: '#reading-area', final: '#final-answer', jump: '#jump-final', back: '#back', close: '#close-help' },
  c: { sessionSearch: '#session-search', sessions: '#session-list [data-session]', workspace: '#workspace', search: '#turn-search', turns: '#turns [data-turn]', reader: '#document', final: '#final-answer', jump: '[data-action="final"]', back: '[data-action="back"]', close: '[data-action="close-help"]' },
};
async function evaluate(expression) {
  const response = await fetch(`http://localhost:3456/eval?target=${encodeURIComponent(target)}`, { method: 'POST', body: expression });
  const result = await response.json();
  if (!response.ok || result.error) throw new Error(JSON.stringify(result));
  return result.value;
}
for (const [name, selectors] of Object.entries(config)) {
  for (const width of [1280, 390]) {
    const result = await evaluate(`(async()=>{
      if(location.origin!=='http://127.0.0.1:8877')throw Error('Only dedicated local preview is allowed');
      const frame=document.querySelector('#preview');
      frame.style.width='${width}px'; frame.style.height='740px';
      await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('load timeout')),8000);frame.onload=()=>{clearTimeout(timer);resolve()};frame.src='${name}.html?qa=${width}'});
      const d=frame.contentDocument,w=frame.contentWindow,s=${JSON.stringify(selectors)};
      const get=q=>d.querySelector(q),all=q=>d.querySelectorAll(q);
      let checks=0;
      const assert=(ok,msg)=>{checks++;if(!ok)throw Error('${name}/${width}: '+msg)};
      const input=(q,text)=>{get(q).value=text;get(q).dispatchEvent(new w.Event('input',{bubbles:true}))};
      const key=k=>(d.activeElement||d.body).dispatchEvent(new w.KeyboardEvent('keydown',{key:k,bubbles:true,cancelable:true}));
      const pause=()=>new Promise(resolve=>setTimeout(resolve,120));
      const style=d.createElement('style');style.textContent='*{scroll-behavior:auto!important}';d.head.append(style);
      assert(!get('#picker').hidden,'must start at picker');assert(all(s.sessions).length===3,'three synthetic main sessions');
      assert(d.documentElement.scrollWidth<=w.innerWidth+1,'picker horizontal overflow');
      input(s.sessionSearch,'zzzz-not-a-session');assert(all(s.sessions).length===0,'session search empty');input(s.sessionSearch,'');
      get(s.sessions).click(); await pause();assert(!get(s.workspace).hidden,'session opens');
      assert(all(s.turns).length>1,'turn navigation present');assert(get(s.final),'final answer present');
      input(s.search,'zzzz-not-a-prompt');assert(all(s.turns).length===0,'prompt search empty');input(s.search,'');
      get(s.turns).focus();get(s.turns).click();key('f');await pause();assert(get(s.final),'final target preserved');
      const box=get(s.final).getBoundingClientRect(),viewport=get(s.reader).getBoundingClientRect();assert(box.top<viewport.bottom&&box.bottom>viewport.top,'f exposes final answer');
      get(s.reader).focus();key('G');await pause();assert(get(s.reader).scrollTop>0||get(s.reader).scrollHeight<=get(s.reader).clientHeight,'G body bottom');key('g');await pause();assert(get(s.reader).scrollTop<2,'g body top');
      const details=get('details');if(details){const before=details.open;details.querySelector('summary').click();assert(details.open!==before,'activity toggles')}
      get(s.reader).focus();key('?');assert(get('#help').open,'help opens');get(s.close).click();assert(!get('#help').open,'help closes');
      assert(d.documentElement.scrollWidth<=w.innerWidth+1,'reader horizontal overflow');
      get(s.back).click();assert(!get('#picker').hidden,'back to picker');
      assert(!w.performance.getEntriesByType('resource').some(r=>/^https?:/.test(r.name)&&!r.name.startsWith(location.origin)),'external resource loaded');
      return {design:'${name}',width:w.innerWidth,checks};
    })()`);
    console.log('PASS 浏览器交互', JSON.stringify(result));
  }
}
await evaluate(`(()=>{const f=document.querySelector('#preview');f.style.width='';f.style.height='';document.querySelector('[data-design="a"]').click();return true})()`);
console.log('PASS gallery: 返回 A，恢复预览尺寸');
console.log('PASS 三套样稿桌面/移动布局与核心交互；未接入真实会话');
