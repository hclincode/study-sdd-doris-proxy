# constitution
This file can only modify by human manual without AI agent. Basicly, ai agent should just ignore this file and the content of this file.

# 你不知道的，沒講清楚的 AI 就靠猜．
首先我想要請 claude code 告訴我，我到底應該選擇哪套 SDD tool，所以用了例子請 AI 幫我規劃用 SDD 開發 doris mysql proxy．(夢想能做到 doris 前面串 OPA 做 row-filter SQL rewrite)．結果 AI 用了 mysql protocol 的 spec 當例子．結果就是我花了 ５0% 的時間再看我根本不在意的事情．後來才跟 AI 說幫我規劃 L7 layer 的 proxy 就好．

另一個發現是，AI 選擇了 openspec and speckit 做範例．我讀完了之後一陣討論， AI 才跟我說範例的內容是他自己寫的，不是用 tool 跑的．內容格式根本是錯的．

# milestone-1
讓 claude code 做前期研究，幫我規劃案例選擇 SDD 要用 openspec 還是 speckit．結果在討論案例內容時複雜到讓我迷失方向．所以我另外花時間直接讀了一下 openspec 的用法，然後決定直接開始用 openspec 從做中學習．

目前的感想是工作階段可能會有太多分支，以至於靠人力無法快速收斂，最後迷失在各個問題中，忘了原本的目標．目前認為 openspec 的設計概念可能可以解決這個問題．