package v2

import (
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
const MaxIdleConns = 4096

type openAPIv2 struct {
    token   *token.Token
    timeout time.Duration

    sandbox     bool   // 璇锋眰娌欑鐜
    debug       bool   // debug 妯″紡锛岃皟璇晄dk鏃跺€欎娇鐢?
    lastTraceID string // lastTraceID id

    restyClient *resty.Client // resty client 澶嶇敤
}

// Setup 娉ㄥ唽
func Setup() {
    openapi.Register(openapi.APIv2, &openAPIv2{})
}

// Version 鍒涘缓褰撳墠鐗堟湰
func (o *openAPIv2) Version() openapi.APIVersion {
    return openapi.APIv2
}

// TraceID 鑾峰彇 lastTraceID id
func (o *openAPIv2) TraceID() string {
    return o.lastTraceID
}

// Setup 鐢熸垚涓€涓疄渚?
func (o *openAPIv2) Setup(token *token.Token, inSandbox bool) openapi.OpenAPI {
    api := &openAPIv2{
        token:   token,
        timeout: 3 * time.Second,
        sandbox: inSandbox,
    }
    api.setupClient() // 鍒濆鍖栧彲澶嶇敤鐨?client
    return api
}

// WithTimeout 璁剧疆璇锋眰鎺ュ彛瓒呮椂鏃堕棿
func (o *openAPIv2) WithTimeout(duration time.Duration) openapi.OpenAPI {
    o.restyClient.SetTimeout(duration)
    return o
}

// Transport 閫忎紶璇锋眰
func (o *openAPIv2) Transport(ctx context.Context, method, url string, body interface{}) ([]byte, error) {
    resp, err := o.request(ctx).SetBody(body).Execute(method, url)
    return resp.Body(), err
}

// 鍒濆鍖?client
func (o *openAPIv2) setupClient() {
    o.restyClient = resty.New().
        SetTransport(createTransport(nil, MaxIdleConns)). // 鑷畾涔?transport
        SetLogger(log.DefaultLogger).
        SetDebug(o.debug).
        SetTimeout(o.timeout).
        SetAuthScheme(string(o.token.Type)).
        SetHeader("User-Agent", version.String()).
        SetHeader("X-Union-Appid", fmt.Sprint(o.token.GetAppID())).
        SetHeader("Accept-Encoding", "gzip").
        SetRetryCount(3).
        SetRetryWaitTime(5 * time.Second).
        SetRetryMaxWaitTime(20 * time.Second).
        // resty 榛樿鐢?json.Marshal 搴忓垪鍖栬姹備綋锛屼細鎶?&銆?銆? 杞箟鎴?
        // \u0026/\u003c/\u003e锛涘甫 OSS 绛惧悕鐨勫浘鐗?URL 鍥犳涓庡師鏂囦笉涓€鑷淬€?
        // 鍏抽棴 HTML 杞箟锛堜粛鏄悎娉?JSON锛岃В鏋愯涔変笉鍙橈級銆?
        SetJSONEscapeHTML(false).
        SetPreRequestHook(
            func(client *resty.Client, request *http.Request) error {
                return openapi.DoReqFilterChains(request, nil)
            },
        ).
        OnBeforeRequest(
            func(c *resty.Client, r *resty.Request) error {
                c.SetAuthScheme("QQBot")
                c.SetAuthToken(o.token.GetAccessToken())
                return nil
            },
        ).
        OnAfterResponse(
            func(client *resty.Client, resp *resty.Response) error {
                log.Infof("%v", respInfo(resp))
                if err := openapi.DoRespFilterChains(resp.Request.RawRequest, resp.RawResponse); err != nil {
                    return err
                }
                traceID := resp.Header().Get(openapi.TraceIDKey)
                o.lastTraceID = traceID
                if !openapi.IsSuccessStatus(resp.StatusCode()) {
                    return errs.New(resp.StatusCode(), string(resp.Body()), traceID)
                }
                return nil
            },
        )
}

// request 姣忎釜璇锋眰锛岄兘闇€瑕佸垱寤轰竴涓?request
func (o *openAPIv2) request(ctx context.Context) *resty.Request {
    return o.restyClient.R().SetContext(ctx)
}

// respInfo 鐢ㄤ簬杈撳嚭鏃ュ織鐨勬椂鍊欐牸寮忓寲鏁版嵁
func respInfo(resp *resty.Response) string {
    bodyJSON, _ := json.Marshal(resp.Request.Body)
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

// reqInfo 鐢ㄤ簬璇锋眰鏃ュ織鏍煎紡鍖?
func reqInfo(req *resty.Request) string {
    bodyJSON, _ := json.Marshal(req)
    return fmt.Sprintf("[OPENAPI]%v %v data:%v", req.Method, req.URL, string(bodyJSON))
}

func createTransport(localAddr net.Addr, idleConns int) *http.Transport {
    dialer := &net.Dialer{
        Timeout:   60 * time.Second,
        KeepAlive: 60 * time.Second,
        Resolver: &net.Resolver{
            PreferGo: true,
            Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
                d := net.Dialer{
                    Timeout: 5 * time.Second,
                }
                return d.DialContext(ctx, network, address)
            },
        },
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
