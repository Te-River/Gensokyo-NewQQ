package handlers

import (
	"encoding/json"
	"runtime"

	"github.com/hoshinonyaruko/gensokyo/buildinfo"
	"github.com/hoshinonyaruko/gensokyo/callapi"
	"github.com/hoshinonyaruko/gensokyo/mylog"
	"github.com/tencent-connect/botgo/openapi"
)

type VersionInfoResponse struct {
	Data    VersionData `json:"data"`
	Message string      `json:"message"`
	RetCode int         `json:"retcode"`
	Status  string      `json:"status"`
	Echo    interface{} `json:"echo"`
}

type VersionData struct {
	AppFullName              string `json:"app_full_name"`
	AppName                  string `json:"app_name"`
	AppVersion               string `json:"app_version"`
	CoolQDirectory           string `json:"coolq_directory"`
	CoolQEdition             string `json:"coolq_edition"`
	GoCQHTTP                 bool   `json:"go-cqhttp"`
	PluginBuildConfiguration string `json:"plugin_build_configuration"`
	PluginBuildNumber        int    `json:"plugin_build_number"`
	PluginVersion            string `json:"plugin_version"`
	ProtocolName             int    `json:"protocol_name"`
	ProtocolVersion          string `json:"protocol_version"`
	RuntimeOS                string `json:"runtime_os"`
	RuntimeVersion           string `json:"runtime_version"`
	Version                  string `json:"version"`
}

func init() {
	callapi.RegisterHandler("get_version_info", GetVersionInfo)
}

func GetVersionInfo(client callapi.Client, api openapi.OpenAPI, apiv2 openapi.OpenAPI, message callapi.ActionMessage) (string, error) {

	var response VersionInfoResponse

	response.Data = VersionData{
		AppFullName:              "gensokyo",
		AppName:                  "gensokyo",
		AppVersion:               buildinfo.Version(),
		CoolQDirectory:           "",
		CoolQEdition:             "pro",
		GoCQHTTP:                 true,
		PluginBuildConfiguration: buildinfo.Version(),
		PluginBuildNumber:        0,
		PluginVersion:            buildinfo.Version(),
		ProtocolName:             4,
		ProtocolVersion:          "v11",
		RuntimeOS:                runtime.GOOS,
		RuntimeVersion:           runtime.Version(),
		Version:                  buildinfo.Version(),
	}
	response.Message = ""
	response.RetCode = 0
	response.Status = "ok"
	response.Echo = message.Echo

	// Convert the members slice to a map
	outputMap := structToMap(response)

	mylog.Printf("get_version_info: %+v\n", outputMap)

	err := client.SendMessage(outputMap)
	if err != nil {
		mylog.Printf("Error sending message via client: %v", err)
	}
	//把结果从struct转换为json
	result, err := json.Marshal(response)
	if err != nil {
		mylog.Printf("Error marshaling data: %v", err)
		//todo 符合onebotv11 ws返回的错误码
		return "", nil
	}
	return string(result), nil
}
