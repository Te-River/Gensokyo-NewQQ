// Package v1 鏄?openapi v1 鐗堟湰鐨勫疄鐜般€?
package v1

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"time"

	"github.com/go-resty/resty/v2" // resty 鏄竴涓紭绉€鐨?rest api 瀹㈡埛绔紝鍙互鏋佸ぇ鐨勫噺灏戝紑鍙戝熀浜?rest 鏍囧噯鎺ュ彛姹傝姹傜殑灏佽宸ヤ綔閲?

	"github.com/tencent-connect/botgo/errs"
	"github.com/tencent-connect/botgo/log"
	"github.com/tencent-connect/botgo/openapi"
	"github.com/tencent-connect/botgo/token"
	"github.com/tencent-connect/botgo/version"
)

// MaxIdleConns 榛樿鎸囧畾绌洪棽杩炴帴姹犲ぇ灏?
const MaxIdleConns = 3000

type openAPI struct {
	token   *token.Token
	timeout time.Duration

	sandbox     bool   // 璇锋眰娌欑鐜
	debug       bool   // debug 妯″紡锛岃皟璇晄dk鏃跺€欎娇鐢?
	lastTraceID string // lastTraceID id

	restyClient *resty.Client // resty client 澶嶇敤
}

// Setup 娉ㄥ唽
func Setup() {
	openapi.Register(openapi.APIv1, &openAPI{})
}

// Version 鍒涘缓褰撳墠鐗堟湰
func (o *openAPI) Version() openapi.APIVersion {
	return openapi.APIv1
}

// TraceID 鑾峰彇 lastTraceID id
func (o *openAPI) TraceID() string {
	return o.lastTraceID
}

// Setup 鐢熸垚涓€涓疄渚?
func (o *openAPI) Setup(token *token.Token, inSandbox bool) openapi.OpenAPI {
	// 鎵撳嵃浼犲叆鐨勪护鐗屼互杩涜璋冭瘯
	//log.Printf("Setup called with token: %+v\n", token)

	api := &openAPI{
		token:   token,
		timeout: 3 * time.Second,
		sandbox: inSandbox,
	}
	api.setupClient() // 鍒濆鍖栧彲澶嶇敤鐨?client
	return api
}

// WithTimeout 璁剧疆璇锋眰鎺ュ彛瓒呮椂鏃堕棿
func (o *openAPI) WithTimeout(duration time.Duration) openapi.OpenAPI {
	o.restyClient.SetTimeout(duration)
	return o
}

// Transport 閫忎紶璇锋眰
func (o *openAPI) Transport(ctx context.Context, method, url string, body interface{}) ([]byte, error) {
	resp, err := o.request(ctx).SetBody(body).Execute(method, url)
	return resp.Body(), err
}

// 鍒濆鍖?client
func (o *openAPI) setupClient() {
	o.restyClient = resty.New().
		SetTransport(createTransport(nil, MaxIdleConns)). // 鑷畾涔?transport
		SetLogger(log.DefaultLogger).
		SetDebug(o.debug).
		SetTimeout(o.timeout).
		SetAuthScheme(string(o.token.Type)).
		SetHeader("User-Agent", version.String()).
		SetHeader("X-Union-Appid", fmt.Sprint(o.token.GetAppID())).
		// resty 榛樿鐢?json.Marshal 搴忓垪鍖栬姹備綋锛屼細鎶?&銆?銆? 杞箟鎴?
		// \u0026/\u003c/\u003e锛涘甫 OSS 绛惧悕鐨勫浘鐗?URL 鍥犳涓庡師鏂囦笉涓€鑷淬€?
		// 鍏抽棴 HTML 杞箟锛堜粛鏄悎娉?JSON锛岃В鏋愯涔変笉鍙橈級銆?
		SetJSONEscapeHTML(false).
		SetPreRequestHook(
			func(client *resty.Client, request *http.Request) error {
				// 鎵ц璇锋眰鍓嶈繃婊ゅ櫒
				// 鐢变簬鍦?`OnBeforeRequest` 鐨勬椂鍊欙紝request 杩樻病鐢熸垚锛屾墍浠?filter 涓嶈兘浣跨敤锛屾墍浠ユ斁鍒?`PreRequestHook`
				return openapi.DoReqFilterChains(request, nil)
			},
		).
		OnBeforeRequest(
			func(c *resty.Client, r *resty.Request) error {
				// 璁剧疆鎺堟潈鏂规涓?"QQBot"
				c.SetAuthScheme("QQBot")
				//c.SetAuthScheme("Bot")
				//c.SetAuthToken(o.token.GetString_old())
				//t.GetAccessToken() 鏄兢鐨?
				c.SetAuthToken(o.token.GetAccessToken())
				return nil
			},
		).
		// 璁剧疆璇锋眰涔嬪悗鐨勯挬瀛愶紝鎵撳嵃鏃ュ織锛屽垽鏂姸鎬佺爜
		OnAfterResponse(
			func(client *resty.Client, resp *resty.Response) error {
				log.Infof("%v", respInfo(resp))
				// 鎵ц璇锋眰鍚庤繃婊ゅ櫒
				if err := openapi.DoRespFilterChains(resp.Request.RawRequest, resp.RawResponse); err != nil {
					return err
				}
				traceID := resp.Header().Get(openapi.TraceIDKey)
				o.lastTraceID = traceID
				// 闈炴垚鍔熷惈涔夌殑鐘舵€佺爜锛岄渶瑕佽繑鍥?error 渚涜皟鐢ㄦ柟璇嗗埆
				if !openapi.IsSuccessStatus(resp.StatusCode()) {
					return errs.New(resp.StatusCode(), string(resp.Body()), traceID)
				}
				return nil
			},
		)
}

// request 姣忎釜璇锋眰锛岄兘闇€瑕佸垱寤轰竴涓?request
func (o *openAPI) request(ctx context.Context) *resty.Request {
	return o.restyClient.R().SetContext(ctx)
}

// respInfo 鐢ㄤ簬杈撳嚭鏃ュ織鐨勬椂鍊欐牸寮忓寲鏁版嵁
func respInfo(resp *resty.Response) string {
	bodyJSON, _ := marshalNoHTMLEscape(resp.Request.Body)
	return fmt.Sprintf(
		"[OPENAPI]%v %v, traceID:%v, status:%v, elapsed:%v req: %v, resp: %v",
		resp.Request.Method,
		resp.Request.URL,
		resp.Header().Get(openapi.TraceIDKey),
		resp.Status(),
		resp.Time(),
		string(bodyJSON),
		string(resp.Body()),
	)
}

// marshalNoHTMLEscape 日志用序列化：关闭 HTML 转义，避免带签名 URL 的 &
// 被打印成 \u0026（真实请求体由 resty 的 SetJSONEscapeHTML(false) 控制）。
func marshalNoHTMLEscape(v interface{}) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimRight(buf.Bytes(), "\n"), nil
}

// reqInfo 鐢ㄤ簬璇锋眰鏃ュ織鏍煎紡鍖?
func reqInfo(req *resty.Request) string {
	bodyJSON, _ := json.Marshal(req)
	return fmt.Sprintf("[OPENAPI]%v %v data:%v", req.Method, req.URL, string(bodyJSON))
}

func createTransport(localAddr net.Addr, idleConns int) *http.Transport {
	dialer := &net.Dialer{
		Timeout:   60 * time.Second,
		KeepAlive: 60 * time.Second,
	}
	if localAddr != nil {
		dialer.LocalAddr = localAddr
	}
	return &http.Transport{
		Proxy:                 http.ProxyFromEnvironment,
		DialContext:           dialer.DialContext,
		ForceAttemptHTTP2:     true,
		MaxIdleConns:          idleConns,
		IdleConnTimeout:       90 * time.Second,
		TLSHandshakeTimeout:   10 * time.Second,
		ExpectContinueTimeout: 1 * time.Second,
		MaxIdleConnsPerHost:   idleConns,
		MaxConnsPerHost:       idleConns,
	}
}
