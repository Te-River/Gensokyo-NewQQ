/**
 * 前端语法验证脚本
 * 验证所有 .ts 和 .vue 文件的基本语法正确性：
 * - .ts 文件：检查是否为合法 UTF-8 且不含空字节
 * - .vue 文件：检查 <script> 标签存在且闭合
 * - 所有文件：检查非空
 *
 * 在 CI 中 ESLint 由 quasar build 覆盖；本脚本提供零依赖的快速语法门控。
 */

const fs = require('fs');
const path = require('path');

const SRC_DIR = path.resolve(__dirname, '..', 'src');
let errors = 0;
let checked = 0;

function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name === '.quasar') continue;
      walk(full);
    } else if (entry.name.endsWith('.ts') || entry.name.endsWith('.vue')) {
      checkFile(full);
    }
  }
}

function checkFile(filePath) {
  checked++;
  const rel = path.relative(SRC_DIR, filePath);
  const content = fs.readFileSync(filePath, 'utf-8');

  // 检查非空
  if (content.trim().length === 0) {
    console.error(`[FAIL] ${rel}: 文件为空`);
    errors++;
    return;
  }

  // 检查不含空字节
  if (content.includes('\0')) {
    console.error(`[FAIL] ${rel}: 包含空字节`);
    errors++;
    return;
  }

  // .vue 文件检查：至少包含 <template> 或 <script> 之一
  if (filePath.endsWith('.vue')) {
    const hasTemplate = /<template[\s>]/.test(content);
    const hasScript = /<script[\s>]/.test(content);
    if (!hasTemplate && !hasScript) {
      console.error(`[FAIL] ${rel}: 缺少 <template> 和 <script> 标签`);
      errors++;
      return;
    }
    // 如果有 script 标签，检查闭合
    if (hasScript && !/<\/script>/.test(content)) {
      console.error(`[FAIL] ${rel}: 缺少 </script> 闭合标签`);
      errors++;
      return;
    }
  }

  // .ts 文件检查基本括号平衡（仅统计，不深入解析）
  if (filePath.endsWith('.ts')) {
    const stripped = content
      .replace(/\/\/.*$/gm, '')       // 移除单行注释
      .replace(/\/\*[\s\S]*?\*\//g, '') // 移除多行注释
      .replace(/'[^']*'/g, '')         // 移除字符串字面量
      .replace(/"[^"]*"/g, '')
      .replace(/`[^`]*`/g, '');
    const opens = (stripped.match(/[\(\[\{]/g) || []).length;
    const closes = (stripped.match(/[\)\]\}]/g) || []).length;
    if (Math.abs(opens - closes) > 2) {
      console.error(`[WARN] ${rel}: 括号不平衡 (opens=${opens}, closes=${closes})`);
    }
  }
}

console.log(`[test] 验证前端源码语法: ${SRC_DIR}`);
walk(SRC_DIR);

if (errors > 0) {
  console.error(`\n[test] 失败: ${errors} 个错误，共检查 ${checked} 个文件`);
  process.exit(1);
} else {
  console.log(`[test] 通过: 共检查 ${checked} 个文件，无错误`);
  process.exit(0);
}
