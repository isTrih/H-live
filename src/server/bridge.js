import brandIcon from "../brand-icon.png";
import "./server.css";
import {operations,parameters} from "./protocol.js";
let csrf="",instance="",loginPromise;
async function fetchJson(path,body,method="POST") {
  const response=await fetch(`/api/v1/${path}`,{method,credentials:"same-origin",cache:"no-store",headers:{"content-type":"application/json",...(csrf?{"x-h-live-csrf":csrf}:{})},...(method==="GET"?{}:{body:JSON.stringify(body??{})})});
  const data=await response.json();
  if(!response.ok) {const e=new Error(data.error||`管理端请求失败 (${response.status})`);e.status=response.status;throw e;}
  return data;
}
async function credentials() {
  let initialError="";
  try {const s=await fetchJson("session",null,"GET");csrf=s.csrf;instance=s.instance;return;}
  catch(e){if(e.status!==401)initialError="暂时无法连接管理接口，请稍后重试。";}
  const root=document.createElement("div");root.className="server-login";
  root.innerHTML=`<form class="server-login-card"><img src="${brandIcon}" alt=""><h1>氢看直播</h1><p>登录服务器管理端</p><label for="server-token">管理令牌</label><input id="server-token" name="token" type="password" autocomplete="off" required maxlength="100" autofocus><div class="server-login-error" role="alert"></div><button class="primary" type="submit">登录</button></form>`;
  document.body.append(root);root.querySelector("input").focus();
  root.querySelector(".server-login-error").textContent=initialError;
  await new Promise(resolve=>root.querySelector("form").addEventListener("submit",async event=>{
    event.preventDefault();const button=root.querySelector("button");button.disabled=true;
    try {const input=root.querySelector("input");const token=input.value.trim();const s=await fetchJson("login",{token});input.value="";csrf=s.csrf;resolve();root.remove();}
    catch(e){root.querySelector(".server-login-error").textContent=e.message;}
    finally{button.disabled=false;}
  }));
  const s=await fetchJson("session",null,"GET");instance=s.instance;
}
export async function connect(){if(!loginPromise)loginPromise=credentials().finally(()=>{loginPromise=null;});await loginPromise;}
export async function bridge(method,params={}){
  const path=operations[method];if(!path)throw new Error("服务端不支持此操作");
  const p=parameters(method,params,instance);
  let result;
  try{result=await fetchJson(path,p);}catch(e){if(e.status!==401)throw e;await connect();result=await fetchJson(path,parameters(method,params,instance));}
  if(result?.instance)instance=result.instance;
  return result;
}
export async function invoke(command,params={}){
  if(command==="choose_export_path")return params.defaultName;
  if(command==="open_live_room"){
    const rid=String(params.rid??"");if(!/^[1-9]\d{0,63}$/.test(rid))throw new Error("直播间 ID 无效");
    window.open(`https://live.douyin.com/${rid}`,"_blank","noopener,noreferrer");return;
  }
  throw new Error("此操作仅适用于桌面版");
}
export async function logout(){await fetchJson("logout",{});csrf="";location.reload();}
