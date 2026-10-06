"use client";

// /budget route — single-page budget + payment hub for the household.
//
// This route exposes the four-action dashboard the household uses to manage
// money together: total SOL balance + approve + reward + track + reimburse.
// Reachable whether or not a wallet is connected (the read-side tiles work
// read-only with just an admin address; the action tiles gate themselves on
// `isOwnerConnected`). The admin-address field on the page itself lets a
// member point at any household without bouncing back to /.
//
// Layout mirrors /scan and /shelf: back-to-dashboard link, title, wallet
// control. Body is the lazy-free `BudgetDashboard` client component.

import { BudgetDashboard } from "@/components/budget/BudgetDashboard";

export default function BudgetPage() {
  return <BudgetDashboard />;
}
