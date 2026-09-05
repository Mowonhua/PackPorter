# PackPorter 应用图标

草地方块包裹表达 Minecraft 整合包，浅色右箭头表达迁移；透明背景适配浅色和深色桌面。

- `packporter-source.png`：内置 imagegen 生成的原始素材（1254 × 1254），保留透明通道。
- `packporter.png`：256 × 256 PNG，用于 Slint 窗口图标和自绘标题栏，编译时嵌入程序。
- `packporter.ico`：Windows EXE 图标，包含 16、20、24、32、40、48、64、128、256 像素的 32 位透明图像，由构建脚本嵌入。

PNG 和 ICO 从同一原图以高质量双三次插值缩小；ICO 各尺寸帧采用 PNG 编码。更换图标时同步更新三个文件，再执行 `cargo build --release`。发布只需分发编译后的程序，不需额外携带这些图片。

## 生成记录

使用内置 imagegen，提示词如下：

```text
Use case: logo-brand. Create one polished production desktop application icon for PackPorter, a Minecraft modpack migration utility. Single isolated icon, 1024x1024 square canvas, truly transparent background. Subject: a compact isometric parcel cube, moss/grass green top, deep forest green left face, medium emerald right face. A single bold warm ivory right-pointing arrow is integrated across the front visible faces, communicating safe transfer to a new pack. Chunky voxel-inspired geometric silhouette, subtle beveled edges, expertly balanced minimal app icon, mostly flat solid color surfaces, crisp clean edges, no fine texture. Cube occupies 82 percent of canvas with balanced transparent margins. Keep arrow extremely simple thick and legible at 24 pixels. No text, no letters, no numbers, no border, no enclosing rounded-square tile, no scenery, no ground plane, no floating particles, no external cast shadow, no additional objects, no Minecraft logos. Deliver finished icon only.
```
