// Small structural DOM for timeline integration tests. It models node identity,
// connection, focus, composition cancellation and scroll; it does not model layout.
const VOID=new Set(["area","base","br","col","embed","hr","img","input","link","meta","param","source","track","wbr"]);
const decode=value=>String(value).replace(/&(?:amp|lt|gt|quot|#39);/g,entity=>({"&amp;":"&","&lt;":"<","&gt;":">","&quot;":'"',"&#39;":"'"})[entity]);
const encode=value=>String(value).replaceAll("&","&amp;").replaceAll('"',"&quot;").replaceAll("<","&lt;").replaceAll(">","&gt;");

class TestNode {
  constructor(type,ownerDocument){this.nodeType=type;this.ownerDocument=ownerDocument;this.parentNode=null;this.childNodes=[];this.listeners=new Map();this.detachments=0;}
  get parentElement(){return this.parentNode?.nodeType===1?this.parentNode:null;}
  get children(){return this.childNodes.filter(child=>child.nodeType===1);}
  get firstChild(){return this.childNodes[0]||null;}
  get firstElementChild(){return this.children[0]||null;}
  get nextSibling(){return this.parentNode?.childNodes[this.parentNode.childNodes.indexOf(this)+1]||null;}
  get isConnected(){let at=this;while(at.parentNode)at=at.parentNode;return at.nodeType===9;}
  contains(node){return node===this||this.childNodes.some(child=>child.contains(node));}
  append(...nodes){for(const node of nodes)this.insertBefore(typeof node==="string"?this.ownerDocument.createTextNode(node):node,null);}
  appendChild(node){this.append(node);return node;}
  insertBefore(node,before){
    if(node.nodeType===11){for(const child of [...node.childNodes])this.insertBefore(child,before);return node;}
    if(node===before)return node;if(node.parentNode)node.remove();
    const index=before?this.childNodes.indexOf(before):this.childNodes.length;if(index<0)throw Error("insertBefore target is not a child");
    this.childNodes.splice(index,0,node);node.parentNode=this;return node;
  }
  remove(){
    if(!this.parentNode)return;
    if(this.isConnected){
      const visit=node=>{node.detachments++;if(node.composing){node.composing=false;node.compositionCancelled=true;}for(const child of node.childNodes)visit(child);};visit(this);
      if(this.contains(this.ownerDocument.activeElement))this.ownerDocument.activeElement=this.ownerDocument.body;
    }
    const parent=this.parentNode;parent.childNodes.splice(parent.childNodes.indexOf(this),1);this.parentNode=null;
  }
  removeChild(node){if(node.parentNode!==this)throw Error("removeChild target is not a child");node.remove();return node;}
  replaceChildren(...nodes){for(const child of [...this.childNodes])child.remove();this.append(...nodes);}
  replaceWith(...nodes){const parent=this.parentNode;if(!parent)return;for(const node of nodes)parent.insertBefore(node,this);this.remove();}
  addEventListener(type,listener){if(!this.listeners.has(type))this.listeners.set(type,new Set());this.listeners.get(type).add(listener);}
  removeEventListener(type,listener){this.listeners.get(type)?.delete(listener);}
  dispatchEvent(event){
    event.target ||= this;event.currentTarget=this;
    event.preventDefault ||= ()=>{event.defaultPrevented=true;};event.stopPropagation ||= ()=>{event.stopped=true;};
    if(event.type==="compositionstart")this.composing=true;if(event.type==="compositionend")this.composing=false;
    for(const listener of this.listeners.get(event.type)||[])listener.call(this,event);
    if(event.bubbles&&!event.stopped)this.parentNode?.dispatchEvent(event);return !event.defaultPrevented;
  }
  get textContent(){return this.nodeType===3?this.data:this.childNodes.map(child=>child.textContent).join("");}
  set textContent(value){if(this.nodeType===3)this.data=String(value);else this.replaceChildren(this.ownerDocument.createTextNode(String(value)));}
  querySelectorAll(selector){const selectors=selector.split(",").map(value=>value.trim()),found=[];const visit=node=>{for(const child of node.children){if(selectors.some(value=>child.matches(value)))found.push(child);visit(child);}};visit(this);return found;}
  querySelector(selector){return this.querySelectorAll(selector)[0]||null;}
}
class TestElement extends TestNode {
  constructor(tag,document){super(1,document);this.localName=tag.toLowerCase();this.tagName=this.localName.toUpperCase();this.attrs=new Map();this.style={};this.scrollTop=0;this.scrollLeft=0;this.clientWidth=1080;this.clientHeight=300;this.scrollHeight=1800;this.selectionStart=0;this.selectionEnd=0;this.selectionDirection="none";this._value=undefined;}
  get attributes(){return [...this.attrs].map(([name,value])=>({name,value}));}
  get id(){return this.getAttribute("id")||"";}set id(value){this.setAttribute("id",value);}
  get className(){return this.getAttribute("class")||"";}set className(value){this.setAttribute("class",value);}
  get classList(){const self=this;return {contains:name=>self.className.split(/\s+/).includes(name),add(...names){self.className=[...new Set([...self.className.split(/\s+/).filter(Boolean),...names])].join(" ");},remove(...names){self.className=self.className.split(/\s+/).filter(name=>!names.includes(name)).join(" ");},toggle(name,force){const present=this.contains(name),add=force===undefined?!present:Boolean(force);if(add)this.add(name);else this.remove(name);return add;}};}
  get dataset(){return new Proxy({},{get:(_,key)=>this.getAttribute(`data-${String(key).replace(/[A-Z]/g,char=>`-${char.toLowerCase()}`)}`)??undefined,set:(_,key,value)=>{this.setAttribute(`data-${String(key).replace(/[A-Z]/g,char=>`-${char.toLowerCase()}`)}`,value);return true;}});}
  get value(){return this._value??(this.localName==="textarea"?this.textContent:this.getAttribute("value")||"");}set value(value){this._value=String(value);}
  get hidden(){return this.hasAttribute("hidden");}set hidden(value){if(value)this.setAttribute("hidden","");else this.removeAttribute("hidden");}
  get disabled(){return this.hasAttribute("disabled");}set disabled(value){if(value)this.setAttribute("disabled","");else this.removeAttribute("disabled");}
  get checked(){return this.hasAttribute("checked");}set checked(value){if(value)this.setAttribute("checked","");else this.removeAttribute("checked");}
  setAttribute(name,value){this.attrs.set(name,String(value));}
  getAttribute(name){return this.attrs.get(name)??null;}
  hasAttribute(name){return this.attrs.has(name);}
  removeAttribute(name){this.attrs.delete(name);}
  focus(){this.ownerDocument.activeElement=this;this.dispatchEvent({type:"focusin",bubbles:true});}
  blur(){if(this.ownerDocument.activeElement===this)this.ownerDocument.activeElement=this.ownerDocument.body;this.dispatchEvent({type:"focusout",bubbles:true});}
  setSelectionRange(start,end,direction="none"){this.selectionStart=start;this.selectionEnd=end;this.selectionDirection=direction;}
  closest(selector){for(let at=this;at?.nodeType===1;at=at.parentElement)if(at.matches(selector))return at;return null;}
  matches(selector){
    const chain=selector.trim().split(/\s+(?![^\[]*\])/);if(chain.length>1){if(!this.matches(chain.pop()))return false;let parent=this.parentElement;while(chain.length){const part=chain.pop();while(parent&&!parent.matches(part))parent=parent.parentElement;if(!parent)return false;parent=parent.parentElement;}return true;}
    let simple=selector.trim();const not=[...simple.matchAll(/:not\(([^)]+)\)/g)];if(not.some(match=>this.matches(match[1])))return false;simple=simple.replace(/:not\([^)]+\)/g,"");
    const tag=simple.match(/^[A-Za-z][\w-]*/)?.[0];if(tag&&tag.toLowerCase()!==this.localName)return false;
    for(const match of simple.matchAll(/#([\w-]+)|\.([\w-]+)|\[([^\]=~^$*|\s]+)(?:([~^$*|]?=)["']?([^\]"']*)["']?)?\]/g)){
      if(match[1]&&this.id!==match[1])return false;if(match[2]&&!this.classList.contains(match[2]))return false;
      if(match[3]){const value=this.getAttribute(match[3]);if(value===null)return false;const expected=(match[5]||"").trim();if(match[4]==="="&&value!==expected||match[4]==="^="&&!value.startsWith(expected)||match[4]==="$="&&!value.endsWith(expected)||match[4]==="*="&&!value.includes(expected))return false;}
    }
    return Boolean(tag||simple.startsWith(".")||simple.startsWith("#")||simple.startsWith("[")||simple==="*");
  }
  get innerHTML(){return this.childNodes.map(serialize).join("");}
  set innerHTML(markup){this.replaceChildren(...parse(String(markup),this.ownerDocument).childNodes);}
  get outerHTML(){return serialize(this);}set outerHTML(markup){this.replaceWith(...parse(String(markup),this.ownerDocument).childNodes);}
  getBoundingClientRect(){return {left:0,top:0,width:this.clientWidth,height:this.clientHeight};}
  scrollIntoView(){}
}
class TestTemplate extends TestElement {
  constructor(document){super("template",document);this.content=new TestNode(11,document);}
  get innerHTML(){return this.content.childNodes.map(serialize).join("");}
  set innerHTML(markup){this.content.replaceChildren(...parse(String(markup),this.ownerDocument).childNodes);}
}
function serialize(node){if(node.nodeType===3)return encode(node.data);const attrs=[...node.attrs].map(([name,value])=>` ${name}="${encode(value)}"`).join("");return `<${node.localName}${attrs}>${node.childNodes.map(serialize).join("")}${VOID.has(node.localName)?"":`</${node.localName}>`}`;}
function parse(markup,document){
  const fragment=new TestNode(11,document),stack=[fragment];
  for(const token of markup.match(/<!--[\s\S]*?-->|<[^>]*>|[^<]+/g)||[]){
    if(token.startsWith("<!--"))continue;
    if(token.startsWith("</")){const tag=token.slice(2,-1).trim().toLowerCase();const index=stack.findLastIndex(node=>node.localName===tag);if(index>0)stack.length=index;continue;}
    if(token.startsWith("<")){
      const tag=token.match(/^<([\w:-]+)/)?.[1];if(!tag)continue;const element=document.createElement(tag);
      const attributes=token.slice(tag.length+1).replace(/\/?\s*>$/,"");for(const match of attributes.matchAll(/([^\s=/>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g))element.setAttribute(match[1],decode(match[2]??match[3]??match[4]??""));
      stack.at(-1).append(element);if(!VOID.has(tag.toLowerCase())&&!token.endsWith("/>"))stack.push(element);
    }else stack.at(-1).append(document.createTextNode(decode(token)));
  }
  return fragment;
}
export function createBookmarkDocument(){
  const document=new TestNode(9,null);document.ownerDocument=document;document.hidden=false;document.defaultView={};
  document.createElement=tag=>String(tag).toLowerCase()==="template"?new TestTemplate(document):new TestElement(String(tag),document);
  document.createTextNode=text=>{const node=new TestNode(3,document);node.data=String(text);return node;};
  document.body=document.createElement("body");document.append(document.body);document.activeElement=document.body;
  document.body.innerHTML='<div class="modal-body"><div id="session-timeline"></div></div>';
  return document;
}
