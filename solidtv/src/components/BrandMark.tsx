// SPDX-License-Identifier: AGPL-3.0-only

import type { NodeProps } from "@solidtv/solid";

export default function BrandMark(props: NodeProps & { size?: number }) {
  return <view {...props} src={`${import.meta.env.BASE_URL}brand.png`} width={props.size ?? 56} height={props.size ?? 56} />;
}
