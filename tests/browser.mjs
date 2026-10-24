import http from 'node:http';
import fs from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const {chromium}=await import(pathToFileURL(path.join(process.env.WISP_TEST_JS_MODULES,'playwright/index.mjs')));
const root=path.resolve(process.argv[2]);
const server=http.createServer(async(req,res)=>{try{const p=path.resolve(root,'.'+decodeURIComponent(req.url.split('?')[0]));if(!p.startsWith(root+'/'))throw Error('path');const data=await fs.readFile(p);res.setHeader('content-type',p.endsWith('.wasm')?'application/wasm':p.endsWith('.js')?'text/javascript':'text/html');res.end(data);}catch{res.writeHead(404);res.end();}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({executablePath:process.env.WISP_CHROMIUM,headless:true,args:['--no-sandbox',...(process.env.WISP_CHROMIUM_SINGLE_PROCESS ? ['--single-process','--no-zygote','--disable-gpu'] : [])]});
try{
 const context=await browser.newContext({serviceWorkers:'block'});const requests=[];
 await context.route('**/*',route=>{requests.push(route.request().url());return route.request().url().startsWith(origin+'/')?route.continue():route.abort('internetdisconnected');});
 const page=await context.newPage();page.on('pageerror',e=>console.error(e));await page.goto(origin+'/index.html');
 await page.waitForFunction(()=>document.body?.textContent.includes('PASS')||document.body?.textContent.includes('FAIL'),{timeout:60000});
 const text=await page.locator('body').textContent();if(!text.includes('PASS browser'))throw Error(text);
 const denied=await page.evaluate(()=>fetch('https://wisp.example/negative-control').then(()=>false,()=>true));if(!denied)throw Error('network control failed');
 console.log(text, JSON.stringify(requests));
 await fs.writeFile(path.join(root,'browser-evidence.json'),JSON.stringify({text,requests,externalControlDenied:denied},null,2));
}finally{await browser.close();server.close();}
