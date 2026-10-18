package handlers

import (
	"errors"
	"strconv"
	"strings"
	"testing"
)

// TestQQErrorCodesFormatKnown 断言新增错误码均能输出官方描述与排查建议
func TestQQErrorCodesFormatKnown(t *testing.T) {
	cases := []struct {
		name        string
		code        int
		description string
		suggestion  string
	}{
		// 富媒体上传
		{"media_unsupported_operation", 10000, "不支持的操作", "请检查请求参数"},
		{"media_group_muted", 850018, "群被禁言或者机器人被禁言", "请检查机器人是否被禁言"},
		{"media_unsupported_format", 850019, "不支持的文件格式", "请检查 file_type 是否正确"},
		{"media_download_failed", 850026, "下载原始文件失败", "请检查 URL 是否可访问或重试"},
		{"media_send_timeout", 850027, "发送数据超时", "请稍后重试"},
		{"media_too_large", 850031, "上传文件超过大小限制", "请减小文件大小"},
		{"media_upload_failed", 40093001, "文件上传失败，请重试", "分片转存 BDH 通道异常，请重试"},
		{"media_daily_quota", 40093002, "超过今天发送文件容量上限", "请明天再试或减少文件大小"},
		// 群聊 v2 撤回
		{"retract_invalid_param", 40061001, "请求参数无效", "请检查请求参数格式"},
		{"retract_no_permission", 40062003, "无操作权限", "请检查机器人是否有操作权限，机器人是否为群管理员或者发消息的用户是否为普通用户"},
		{"retract_time_limit", 40064004, "已超出消息撤回时限", "消息发送超过2分钟后不可撤回"},
		{"retract_failed", 50065001, "消息撤回失败，请稍后重试", "请稍后重试"},
		// 获取访问凭证
		{"token_too_many_requests", 100001, "Too many requests", "请求过于频繁，请降低调用频率后重试"},
		{"token_appid_invalid", 100007, "appid invalid", "AppID 无效，或机器人状态不正常（被封禁或已删除），请检查 AppID 是否正确以及机器人状态"},
		{"token_invalid_secret", 100016, "invalid appid or secret", "AppID 或 ClientSecret 不正确，请检查传入的 appId 和 clientSecret 是否与开放平台管理端一致"},
		// 应用权限（11253 已按官方措辞更新）
		{"app_privilege_not_pass", 11253, "应用无接口访问权限", "该接口仅白名单机器人可用，请联系平台运营申请权限"},
		// 10004 保留频道语义（取 token 场景含义见映射表注释）
		{"unknown_guild", 10004, "UnknownGuild 频道异常", "请检查频道ID"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := errors.New(`PostGroupMessage failed: {"code":` + strconv.Itoa(tc.code) + `,"message":"mock"}`)
			got := FormatQQError(err)
			if !strings.Contains(got, tc.description) {
				t.Errorf("code %d: 输出缺少描述 %q，实际：%s", tc.code, tc.description, got)
			}
			if !strings.Contains(got, tc.suggestion) {
				t.Errorf("code %d: 输出缺少建议 %q，实际：%s", tc.code, tc.suggestion, got)
			}
			if !strings.Contains(got, strconv.Itoa(tc.code)) {
				t.Errorf("code %d: 输出缺少错误码，实际：%s", tc.code, got)
			}
		})
	}
}

// TestQQErrorCodesFormatUnknown 断言未收录的错误码仍走"未识别错误码"分支
func TestQQErrorCodesFormatUnknown(t *testing.T) {
	got := FormatQQError(errors.New(`{"code":99999999,"message":"mock"}`))
	if !strings.Contains(got, "未识别错误码") {
		t.Fatalf("未收录错误码应提示未识别，实际：%s", got)
	}
	if !strings.Contains(got, "99999999") {
		t.Fatalf("未收录错误码应回显错误码，实际：%s", got)
	}
}
