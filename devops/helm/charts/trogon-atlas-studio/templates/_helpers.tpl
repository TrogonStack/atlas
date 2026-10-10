{{- define "trogon-atlas-studio.name" -}}
{{- default "trogon-atlas-studio" .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
  Standalone installs want a sane release-derived name. Umbrella installs
  (through the trogon-atlas chart's "studio" alias) want the exact names an
  operator already depends on: release "test" renders "test-trogon-atlas-studio",
  release "trogon-atlas" renders "trogon-atlas-studio".

  1. fullnameOverride wins outright.
  2. A release name that already names this chart is used as-is.
  3. Otherwise anchor on "trogon-atlas": reuse the release name if it already
     carries that, else append it, then suffix "-studio".
*/}}
{{- define "trogon-atlas-studio.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
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

{{/*
  The name of the trogon-atlas-server this studio talks to by default, when
  server.host is left empty. A subchart cannot see a sibling subchart's
  values (or its fullnameOverride), so this replicates the server chart's own
  naming rule from this release's name instead of calling into it, which
  lands on the same name for the common case: both charts installed together
  under one umbrella release, neither overriding fullnameOverride.
*/}}
{{- define "trogon-atlas-studio.serverFullname" -}}
{{- if .Values.server.host }}
{{- .Values.server.host }}
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

{{- define "trogon-atlas-studio.chart" -}}
{{- printf "%s-%s" "trogon-atlas-studio" .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "trogon-atlas-studio.labels" -}}
helm.sh/chart: {{ include "trogon-atlas-studio.chart" . }}
app.kubernetes.io/name: {{ include "trogon-atlas-studio.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "trogon-atlas-studio.selectorLabels" -}}
app.kubernetes.io/name: {{ include "trogon-atlas-studio.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "trogon-atlas-studio.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "trogon-atlas-studio.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "trogon-atlas-studio.image" -}}
{{- $registry := .root.Values.global.imageRegistry }}
{{- $tag := default .root.Chart.AppVersion .image.tag }}
{{- if $registry }}
{{- printf "%s/%s:%s" $registry .image.repository $tag }}
{{- else }}
{{- printf "%s:%s" .image.repository $tag }}
{{- end }}
{{- end }}

{{- define "trogon-atlas-studio.grpcUrl" -}}
{{- default (printf "http://%s:%d" (include "trogon-atlas-studio.serverFullname" .) (int .Values.server.grpcPort)) .Values.config.grpcUrl }}
{{- end }}
