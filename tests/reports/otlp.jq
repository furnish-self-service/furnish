# Report lines (as furnish queues them, slurped with -s) as the OTLP/HTTP JSON request furnish
# sends: the mapping of src/reports.rs (otlp), for tests that post to the receiver directly.
def any: if type == "string" then {stringValue: .}
  elif type == "number" then {intValue: tostring}
  elif type == "boolean" then {boolValue: .}
  elif type == "array" then {arrayValue: {values: map(any)}}
  elif type == "object" then {kvlistValue: {values: [to_entries[] | {key, value: (.value | any)}]}}
  else {} end;
def attr($k; $v): {key: $k, value: ($v | any)};
def names: [["event","furnish.event"],["outcome","furnish.outcome"],["app","furnish.app"],["collection","furnish.collection"],
  ["version","furnish.collection.commit"],["trigger","furnish.trigger"],["error","error.message"],["durationMs","furnish.duration_ms"],
  ["installedVersion","furnish.installed_version"],["offeredVersion","furnish.offered_version"],["apps","furnish.apps"]];
def resource: [attr("service.name"; "furnish"), attr("service.version"; .furnish), attr("os.type"; (if .os == "macos" then "darwin" else .os end)),
  attr("os.version"; .osVersion), attr("device.id"; .device)] + (if .host then [attr("host.name"; .host)] else [] end);
def record: . as $r | {
  timeUnixNano: ((.time | sub("\\.[0-9]+Z$"; "Z") | fromdateiso8601) * 1000000000
  + ((.time | capture("\\.(?<ms>[0-9]{3})Z$").ms // "0") | tonumber) * 1000000 | tostring),
  severityNumber: (if .outcome == "failed" or .outcome == "noneOffered" then 17
    elif .event == "fallback" or .outcome == "notAcceptable" or .outcome == "unknown" then 13 else 9 end),
  severityText: (if .outcome == "failed" or .outcome == "noneOffered" then "ERROR"
    elif .event == "fallback" or .outcome == "notAcceptable" or .outcome == "unknown" then "WARN" else "INFO" end),
  body: {stringValue: (if .apps then "\(.event) of \(.apps | length) apps" else "\(.event) \(.app // .collection) \(.outcome)" end)},
  attributes: [names[] | select($r[.[0]] != null) | attr(.[1]; $r[.[0]])]};
{resourceLogs: [group_by(resource | tostring)[] | {resource: {attributes: (.[0] | resource)}, scopeLogs: [{scope: {name: "furnish"}, logRecords: map(record)}]}]}
