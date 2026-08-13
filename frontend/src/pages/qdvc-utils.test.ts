/**
 * QDVC URI 编解码单元测试
 * 运行方式：npm test（vitest run）
 */
import { describe, it, expect, beforeAll } from 'vitest';

// Node 环境没有 window，但 Node 16+ 提供全局 atob/btoa/TextDecoder
beforeAll(() => {
  (globalThis as unknown as { window?: unknown }).window = globalThis;
});

import { QDVC } from './qdvc-utils';

describe('QDVC.parse / QDVC.stringify', () => {
  it('base64 输出可往返还原 device', () => {
    const uri = QDVC.stringify({ device: '测试设备' }, 'base64');
    expect(uri).toMatch(/^qdvc:/);
    const parsed = QDVC.parse(uri);
    expect(parsed?.device).toBe('测试设备');
    expect(parsed?.session).toBeUndefined();
  });

  it('base16384 输出可往返还原 device', () => {
    const uri = QDVC.stringify({ device: '测试设备' }, 'base16384');
    const parsed = QDVC.parse(uri);
    expect(parsed?.device).toBe('测试设备');
  });

  it('带 session 时往返还原 device 与 session 字节', () => {
    const uri = QDVC.stringify(
      { device: 'device-1', session: 'session-1' },
      'base64'
    );
    const parsed = QDVC.parse(uri);
    expect(parsed?.device).toBe('device-1');
    expect(parsed?.session).toBeInstanceOf(Uint8Array);
    expect(
      new TextDecoder().decode(parsed?.session ?? new Uint8Array())
    ).toBe('session-1');
  });

  it('非法 URI 返回 null', () => {
    expect(QDVC.parse('not-a-qdvc-uri')).toBeNull();
    expect(QDVC.parse('')).toBeNull();
  });
});
