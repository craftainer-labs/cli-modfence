import React from "react";
import { Button } from "./button";

type Props = { label: string };

// modfence: layer=ui
export default function (props: Props) {
  return <Button onClick={() => track(props.label)}>{format(props)}</Button>;
}

export const Label = ({ text }: Props) => <span>{text}</span>;
