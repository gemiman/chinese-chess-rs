import { useEffect, useState } from 'react'

/**
 * 三个页面，用网址里的 `#` 部分区分。
 *
 * # 为什么用 hash 而不是装一个路由库
 *
 * 三个页面、没有嵌套、没有参数，用不上 `react-router` 那套（它还要求把
 * 应用包一层 `<Router>`，并为「动态路由参数」付一份认知成本）。而 `hashchange`
 * 是浏览器自带的事件，**零依赖**就能拿到两个真正有用的能力：
 *
 * - 浏览器/窗口的「后退」键能回到上一页 —— 装路由库之前这是完全没有的；
 * - 刷新页面停在原页，不会被打回第一页。
 *
 * # 为什么用 hash 而不是 pathname
 *
 * 桌面端是 Tauri 把 `frontend/dist` 装进窗口里，用的是 `file://` 协议；
 * 换成 `/setup` 这种路径，刷新时浏览器会去找一个叫 `setup` 的文件，直接 404。
 * hash 不改路径，两种宿主都不会出问题。
 *
 * # 分享链接现在还做不到
 *
 * 网址里只有页面名，**没有对局数据**。所以把 `#/play` 发给别人是没用的 ——
 * 对方打开的只会是一局新棋。分享棋局要等联网对战那一版，届时对局有 ID 可引用。
 */

export type Route = 'setup' | 'play' | 'analysis'

const ROUTES: readonly Route[] = ['setup', 'play', 'analysis']

/** 认不出的 hash 一律回到设置页 —— 宁可多一步，也不要停在一个空白页上。 */
const FALLBACK: Route = 'setup'

function parse(hash: string): Route {
  const name = hash.replace(/^#\/?/, '').split('?')[0]
  return (ROUTES as readonly string[]).includes(name) ? (name as Route) : FALLBACK
}

/**
 * 跳到某个页面。
 *
 * `replace` 会把当前这条历史记录**替换掉**而不是新增一条。
 * 兜底跳转（比如「没有对局却落在 #/play」被送回设置页）必须用它 ——
 * 用默认的 push 会变成死循环：回退到 #/play → 又被推一条 #/setup →
 * 再回退 → 又推一条……用户按后退键永远退不出去。
 */
export function navigate(route: Route, replace = false): void {
  const target = `#/${route}`
  if (window.location.hash === target) return
  if (replace) {
    window.location.replace(target)
  } else {
    window.location.hash = target
  }
}

/** 当前页面。跟着「后退」键与手动改网址走。 */
export function useRoute(): Route {
  const [route, setRoute] = useState<Route>(() => parse(window.location.hash))
  useEffect(() => {
    const sync = () => setRoute(parse(window.location.hash))
    window.addEventListener('hashchange', sync)
    // 首次进入时 hash 可能是空的，补上，让后退键有东西可回
    if (window.location.hash === '') window.location.hash = `#/${FALLBACK}`
    return () => window.removeEventListener('hashchange', sync)
  }, [])
  return route
}
