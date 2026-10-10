{{/*
  NOTES.txt needs the server and studio resource names, but a parent chart
  cannot call into a subchart's named templates (Helm scopes "define" to the
  chart that declares it plus its descendants, not its siblings' parents).
  These replicate the subcharts' own fullname rules so the names match
  exactly: release "test" -> "test-trogon-atlas-server"/"-studio", release
  "trogon-atlas" -> "trogon-atlas-server"/"-studio".
*/}}
{{- define "trogon-atlas.server.fullname" -}}
{{- if .Values.server.fullnameOverride }}
{{- .Values.server.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else if contains "trogon-atlas-server" .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $base := .Release.Name }}
{{- if not (contains "trogon-atlas" .Release.Name) }}
{{- $base = printf "%s-trogon-atlas" .Release.Name }}
{{- end }}
{{- printf "%s-server" $base | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "trogon-atlas.studio.fullname" -}}
{{- if .Values.studio.fullnameOverride }}
{{- .Values.studio.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else if contains "trogon-atlas-studio" .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $base := .Release.Name }}
{{- if not (contains "trogon-atlas" .Release.Name) }}
{{- $base = printf "%s-trogon-atlas" .Release.Name }}
{{- end }}
{{- printf "%s-studio" $base | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
