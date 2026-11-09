import { createFileRoute } from "@tanstack/react-router";
import { WalletApp } from "@/components/shell/WalletApp";

export const Route = createFileRoute("/")({ component: Home });

function Home() {
  return <WalletApp />;
}
