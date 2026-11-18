export function sessionDate(value,withTime=false){
  if(value===null||value===undefined||value==="")return "日期未知";
  const date=new Date(typeof value==="number"?value*1000:value);
  if(!Number.isFinite(date.getTime()))return String(value).slice(0,16).replace("T"," ");
  const parts=Object.fromEntries(new Intl.DateTimeFormat("en-GB",{timeZone:"Asia/Shanghai",year:"2-digit",month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit",hourCycle:"h23"}).formatToParts(date).map(p=>[p.type,p.value]));
  return `${parts.year}/${parts.month}/${parts.day}${withTime?` ${parts.hour}:${parts.minute}`:""}`;
}
