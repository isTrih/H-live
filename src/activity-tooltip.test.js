import test from "node:test";
import assert from "node:assert/strict";
import {createActivityTooltip} from "./activity-tooltip.js";

function fixture(){
  const cards=[],portals=[],listeners=new Map();
  const owner={isConnected:true,querySelectorAll:()=>cards.filter(card=>card.isConnected),closest:()=>null};
  const host={append(portal){portal.parentElement=this;portal.isConnected=true;portals.push(portal);}};
  const view={innerWidth:1000,innerHeight:700,addEventListener:(key,handler)=>listeners.set(`window:${key}`,handler),removeEventListener:key=>listeners.delete(`window:${key}`)};
  const doc={body:host,fullscreenElement:null,addEventListener:(key,handler)=>listeners.set(key,handler),removeEventListener:key=>listeners.delete(key),createElement:()=>({style:{},dataset:{},setAttribute(){},classList:{toggle(){}},querySelectorAll:()=>[],getBoundingClientRect:()=>({width:300,height:180}),remove(){this.isConnected=false;this.removed=true;}})};
  const makeCard=(key,scope,content)=>{
    const classes=new Set(["timeline-activity-interval","activity-density"]),source={innerHTML:content};
    const card={isConnected:true,dataset:{activityKey:key,activity:"99"},classList:{contains:name=>classes.has(name)},querySelector:()=>source,getBoundingClientRect:()=>({left:100,right:150,top:400,bottom:430}),closest:selector=>selector.includes(",")?card:selector==="#session-timeline"?owner:selector===".timeline-integrated-progress"&&scope==="media"?host:null};
    cards.push(card);return card;
  };
  return {controller:createActivityTooltip({document:doc,window:view}),makeCard,portals,listeners};
}

test("density hover refreshes its precise segment without moving to the other progress bar",()=>{
  const f=fixture(),first=f.makeCard("lottery:50:51","media","原密度段");
  assert.equal(f.controller.show(first),true);assert.equal(f.portals[0].innerHTML,"原密度段");
  first.isConnected=false;
  f.makeCard("lottery:50:51","archive","归档面积图");
  f.makeCard("lottery:50:51","media","录播进度条 · 51轮重叠");
  assert.equal(f.controller.refresh(),true);assert.equal(f.portals[0].innerHTML,"录播进度条 · 51轮重叠");
  f.listeners.get("window:scroll")();assert.equal(f.portals[0].removed,true);
  f.controller.destroy();assert.equal(f.listeners.size,0);
});
