export const operations={
 init:"state",poll:"events/poll",save_settings:"settings",recent_messages:"messages/recent",archive_anchors:"archives/anchors",archive_sessions:"archives/sessions",
 preview_archive_delete:"archives/delete-preview",execute_archive_delete:"archives/delete",archive_delete_status:"archives/delete-status",
 add_anchors:"anchors/import",remove:"anchors/remove",rename:"anchors/rename",set_anchor_tags:"anchors/tags",set_pc_unsupported:"anchors/pc-support",start:"monitor/start",stop:"monitor/stop",
 configure_proxy:"proxies/settings",proxy_status:"proxies/status",preview_proxies:"proxies/preview",import_proxies:"proxies/import",list_imported_proxies:"proxies/list",remove_imported_proxies:"proxies/remove",refresh_proxies:"proxies/check",
 list_proxy_subscriptions:"subscriptions/list",add_proxy_subscription:"subscriptions/add",refresh_proxy_subscription:"subscriptions/refresh",remove_proxy_subscription:"subscriptions/remove",
 refresh_viewers:"viewers/refresh",viewer_refresh_status:"viewers/status",quality_sessions:"analysis/sessions",quality_detail:"analysis/detail",quality_settings:"analysis/settings",quality_rebuild:"analysis/rebuild",quality_pause:"analysis/pause",quality_configure:"analysis/configure",quality_commands:"analysis/commands",
 export:"exports/messages",quality_export:"exports/analysis",cancel_export:"exports/cancel",
 session_timeline:"analysis/timeline",highlight_settings:"highlights/settings",highlight_configure:"highlights/configure",highlight_models:"highlights/models",highlight_analyze:"highlights/analyze",highlight_status:"highlights/status",highlight_cancel:"highlights/cancel",
 community_settings:"community/settings",community_configure:"community/configure",session_replay:"replays/session",session_replay_timing:"replays/session/timing",
 mark_list:"marks/list",mark_create:"marks/create",mark_update:"marks/update",mark_delete:"marks/delete",
 time_display_settings:"replays/time-display",time_display_save:"replays/time-display/save",
};
export function parameters(method,params,instance){
 const p={...params};
 if(method==="export"||method==="quality_export"){p.file_name=p.output_path;delete p.output_path;}
 if(method==="poll")p.instance=instance;
 return p;
}
