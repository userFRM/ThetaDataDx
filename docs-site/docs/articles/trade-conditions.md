---
title: Trade Conditions
description: The full ThetaData trade-condition code table for the condition fields on trade rows.
---

# Trade Conditions

Trade rows carry a `condition` code plus `ext_condition1`–`4`. Below is ThetaData's complete published trade-condition set: every code, its per-flag behavior (whether it cancels, is a late/open report, updates volume / high / low / last), and its description. Quote-row condition fields are on the [Quote Conditions](/articles/quote-conditions) page.

[Download as CSV](/csv/trade-conditions.csv)

Legend: ✓ = yes, blank = no, `*` = conditional (applies only in the case noted in the description). Source: ThetaData's [Trade Conditions article](https://docs.thetadata.us/Articles/Errors-Exchanges-Conditions/Trade-Conditions.html).

| Code | Name | Cancel | Late Report | Auto Executed | Open Report | Volume | High | Low | Last | Description |
|---:|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|---|
| 0 | REGULAR |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Regular Trade |
| 1 | FORM_T |  |  |  |  | ✓ |  |  |  | Form T. Before and After Regular Hours. note: NYSE/AMEX previously used code 'T' for BurstBasket. |
| 2 | OUT_OF_SEQ |  | ✓ |  |  | ✓ | ✓ | ✓ | * | Report was sent Out Of Sequence. Updates last if it becomes only trade (if the trade reports before it are canceled, for example). |
| 4 | AVG_PRC_NASDAQ |  |  |  |  | ✓ |  |  |  | Average Price. Nasdaq stocks. Similar to AvgPrc, but does not set high/low/last. |
| 5 | OPEN_REPORT_LATE |  | ✓ |  |  | ✓ | ✓ | ✓ | * | NYSE/AMEX. Market opened Late. Here is the report. It may not be in sequence. Nasdaq uses OpenReportOutOfSeq. *update last if only trade. |
| 6 | OPEN_REPORT_OUT_OF_SEQ |  | ✓ |  |  | ✓ | ✓ | ✓ |  | Report IS out of sequence. Market was open, and now this report is just getting to us. |
| 7 | OPEN_REPORT_IN_SEQ |  | ✓ |  |  | ✓ | ✓ | ✓ | ✓ | Opening report. This is the first price. |
| 8 | PRIOR_REFERENCE_PRICE |  | ✓ |  |  | ✓ | ✓ | ✓ | * | Trade references price established earlier. *Update last if this is the only trade report. |
| 9 | NEXT_DAY_SALE |  |  |  |  | ✓ |  |  |  | NYSE/AMEX:Next Day Clearing. Nasdaq: Delivery of Securities and payment one to four days later.*As of September 5, 2017, the NYSE will no longer accept orders with Cash, Next Day or Seller's Option instructions. |
| 10 | BUNCHED |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Aggregate of 2 or more Regular trades at same price within 60 seconds and each trade size not greater than 10,000. |
| 11 | CASH_SALE |  |  |  |  | ✓ |  |  |  | Delivery of securities and payment on the same day.*As of September 5, 2017, the NYSE will no longer accept orders with Cash, Next Day or Seller's Option instructions. |
| 12 | SELLER |  |  |  |  | ✓ |  |  |  | Stock can be delivered up to 60 days later as specified by the seller. After 1995, the number of days can be greater than 60. note: delivery of 3 days would be considered a regular trade.*As of September 5, 2017, the NYSE will no longer accept orders with Cash, Next Day or Seller's Option instructions. |
| 13 | SOLD_LAST |  | ✓ |  |  | ✓ | ✓ | ✓ | * | Late Reporting. *Sets Consolidated Last if no other qualifying Last, or same Exchange set previous Trade, or Exchange is Listed Exchange. |
| 14 | RULE_127 |  |  |  |  | ✓ | ✓ | ✓ | ✓ | NYSE only. Rule 127 basically denotes the trade was executed as a block trade. |
| 15 | BUNCHED_SOLD |  | ✓ |  |  | ✓ | ✓ | ✓ | * | Several trades were bunched into one trade report, and the report is late. *Update last if this is first trade. |
| 16 | NON_BOARD_LOT |  |  |  |  | ✓ |  |  |  | Size of trade is less than a board lot (oddlot). A board lot is usually 1,00 shares. Note this is Canadian markets. |
| 17 | POSIT |  |  |  |  | ✓ | ✓ | ✓ |  | POSIT Canada is an electronic order matching system that prices trades at the mid-point of the bid and ask in the continuous market. |
| 18 | AUTO_EXECUTION |  |  | ✓ |  | ✓ | ✓ | ✓ | ✓ | Transaction executed electronically. Soley for information. Only found in OPRA -- options trades, and quite common. |
| 19 | HALT |  |  |  |  |  |  |  |  | Temporary halt in trading in a particular security for one or more participants. |
| 20 | DELAYED |  |  |  |  | ✓ |  |  |  | Indicates a delayed opening |
| 21 | REOPEN |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Reopening of a contract that was previously halted. |
| 22 | ACQUISITION |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction on exchange as a result of an Exchange Acquisition |
| 25 | BURST_BASKET |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Specialist bought or sold this stock as part of an execution of a specific basket of stocks. |
| 26 | OPEN_DETAIL |  | ✓ |  |  |  |  |  |  | 107-113, 130, 160 Deleted an existing Sale Condition (Note: the code may be repurposed at a future date): 'G' - 'Opening/Reopening Trade Detail'. This trade is one of several trades that made up the open report trade. Often the open report has a large size which was made up of orders placed overnight. After trading has commenced, the individual trades of the open report trade are sent with this condition. Note it doesn't update volume, high, low, or last because it's already been accounted for in the open report. |
| 27 | INTRA_DETAIL |  | ✓ |  |  |  |  |  |  | This trade is one of several trades that made up a previous trade. Similar to OpenDetail but refers to a trade report that was not the opening trade report. |
| 28 | BASKET_ON_CLOSE |  | ✓ |  |  | ✓ |  |  |  | A trade consisting of a paired basket order to be executed based on the closing value of an index. These trades are reported after the close when the index closing value is known. |
| 29 | RULE_155 |  |  |  |  | ✓ | ✓ | ✓ | ✓ | AMEX only rule 155. Sale of block at one clean-up price. |
| 30 | DISTRIBUTION |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Sale of a large block of stock in a way that price is not adversely affected. |
| 31 | SPLIT |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Execution in 2 markets when the specialist or MM in the market first receiving the order agrees to execute a portion of it at whatever price is realized in another market to which the balance of the order is forwarded for execution. |
| 32 | REGULAR_SETTLE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | RegularSettle |
| 33 | CUSTOM_BASKET_CROSS |  |  |  |  | ✓ |  |  |  | One of two types:2 paired but seperate orders in which a market maker or member facilitates both sides of a remaining portion of a basket. A split basket plus an entire basket where the market maker or member facilitates the remaining shares of the split basket. |
| 34 | ADJ_TERMS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Terms have been adjusted to reflect stock split/dividend or similar event. |
| 35 | SPREAD |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Spread between 2 options in the same options class. |
| 36 | STRADDLE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Straddle between 2 options in the same options class. |
| 37 | BUY_WRITE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | This is the option part of a covered call. |
| 38 | COMBO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A buy and a sell in 2 or more options in the same class. |
| 39 | STPD |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Traded at price agreed upon by the floor following a non-stopped trade of the same series at the same price. |
| 40 | CANC | ✓ |  |  |  |  |  |  |  | Cancel a previously reported trade - it will not be the first or last trade record. note: If the most recent report is Out of seq, SoldLast, or a type that does not qualify to set the last, that report can be considered in processing the cancel. |
| 41 | CANC_LAST | ✓ |  |  |  |  |  |  |  | Cancel the most recent trade report that is qualified to set the last. |
| 42 | CANC_OPEN | ✓ |  |  |  |  |  |  |  | Cancel the opening trade report. |
| 43 | CANC_ONLY | ✓ |  |  |  |  |  |  |  | Cancel the only trade report. There is only one trade report, cancel it. |
| 44 | CANC_STPD | ✓ |  |  |  |  |  |  |  | Cancel the trade report that has the condition STPD. |
| 45 | MATCH_CROSS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | CTS and UTP: Cross Trade. A Cross Trade a trade transaction resulting from a market center's crossing session. |
| 46 | FAST_MARKET |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Term used to define unusually hectic market conditions. |
| 47 | NOMINAL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Nominal price. A calculated price primarily generated to represent the fair market value of an inactive instrument for the purpose of determining margin requirements and evaluating position risk. Common in futures and futures options. |
| 48 | CABINET |  |  |  | ✓ |  |  |  |  | A trade in a deep out-of-the-money option priced at one-half the tick value. Used by options traders to liquidate positions. |
| 49 | BLANK_PRICE |  |  |  |  |  |  |  |  | Sent by an exchange to blank out the associated price (bid, ask or trade). |
| 50 | NOT_SPECIFIED |  |  |  |  |  |  |  |  | An unspecified (generalized) condition. |
| 51 | MC_OFFICIAL_CLOSE |  |  |  |  |  |  |  |  | The Official closing value as determined by a Market Center. |
| 52 | SPECIAL_TERMS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Indicates that all trades executed will be settled in other than the regular manner. |
| 53 | CONTINGENT_ORDER |  |  |  |  | ✓ | ✓ | ✓ | ✓ | The result of an order placed by a Participating Organization on behalf of a client for one security and contingent on the execution of a second order placed by the same client for an offsetting volume of a related security. |
| 54 | INTERNAL_CROSS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A cross between two client accounts of a Participating Organization which are managed by a single firm acting as portfolio manager with discretionary authority to manage the investment portfolio granted by each of the clients. This was originally from Toronto Stock Exchange (TSX). Information located here. |
| 55 | STOPPED_REGULAR |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Stopped Stock  Regular Trade. |
| 56 | STOPPED_SOLD_LAST |  |  |  |  |  | ✓ | ✓ | ✓ | TStopped Stock  SoldLast Trade |
| 58 | BASIS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A transaction involving a basket of securities or an index participation unit that is transacted at prices achieved through the execution of related exchange-traded derivative instruments, which may include index futures, index options and index participation units in an amount that will correspond to an equivalent market exposure. |
| 59 | VWAP |  |  |  |  | ✓ |  |  |  | Volume Weighted Average Price. A transaction for the purpose of executing trades at a volume-weighted average price of the security traded for a continuous period on or during a trading day on the exchange. |
| 60 | SPECIAL_SESSION |  |  |  |  | ✓ |  |  |  | Occurs when an order is placed by a purchase order on behalf of a client for execution in the Special Trading Session at the last sale price. |
| 61 | NANEX_ADMIN |  |  |  |  |  |  |  |  | Used to make volume and price corrections to match official exchange values. |
| 62 | OPEN_REPORT |  |  |  |  | ✓ | ✓ | ✓ |  | Indicates an opening trade report. |
| 63 | MARKET_ON_CLOSE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | The Official closing value as determined by a Market Center. |
| 64 | SETTLE_PRICE |  |  |  |  |  |  |  |  | Settlement price |
| 65 | OUT_OF_SEQ_PRE_MKT |  | ✓ |  |  | ✓ |  |  |  | An out of sequence trade that exectuted in pre or post market -- a combination of FormT and OutOfSeq. |
| 66 | MC_OFFICIAL_OPEN |  |  |  |  |  |  |  |  | Indicates the 'Official' opening value as determined by a Market Center. This transaction report will contain the market center generated opening price. |
| 67 | FUTURES_SPREAD |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Execution was part of a spread with another futures contract. |
| 68 | OPEN_RANGE |  |  |  |  |  | ✓ | ✓ |  | Two trade prices are used to indicate an opening range representing the high and low prices during the first 30 seconds or so of trading. |
| 69 | CLOSE_RANGE |  |  |  |  |  | ✓ | ✓ |  | Two trade prices are used to indicate an opening range representing the high and low prices during the last 30 seconds or so of trading. |
| 70 | NOMINAL_CABINET |  |  |  |  |  |  |  |  | Nominal Cabinet |
| 71 | CHANGING_TRANS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Changing Transaction |
| 72 | CHANGING_TRANS_CAB |  |  |  |  |  |  |  |  | Changing Cabinet Transaction |
| 73 | NOMINAL_UPDATE |  |  |  |  |  |  |  |  | Nominal price update |
| 74 | PIT_SETTLEMENT |  |  |  |  |  |  |  |  | Sent with a "pit session" settlement price to the electronic session, for the purpose of computing net change from the next day electronic session and the prior session settlement price. |
| 75 | BLOCK_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | An executed trade of a large number of shares, typically 10,000 shares or more. |
| 76 | EXG_FOR_PHYSICAL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Exchange Future for Physical |
| 77 | VOLUME_ADJUSTMENT |  |  |  |  | ✓ |  |  |  | An adjustment made to the cumulative trading volume for a trading session. |
| 78 | VOLATILITY_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Volatility trade |
| 79 | YELLOW_FLAG |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Appears when reporting exchnge may be experiencing technical difficulties. |
| 80 | FLOOR_PRICE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Distinguishes a floor Bid/Ask from a member Bid Ask on LME |
| 81 | OFFICIAL_PRICE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Official bid/ask price used by LME. |
| 82 | UNOFFICIAL_PRICE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Unofficial bid/ask price used by LME. |
| 83 | MID_BID_ASK_PRICE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A price halfway between the bid and ask on LME. |
| 84 | END_SESSION_HIGH |  |  |  |  |  | ✓ |  |  | End of Session High Price. |
| 85 | END_SESSION_LOW |  |  |  |  |  |  | ✓ |  | End of Session Low Price. |
| 86 | BACKWARDATION |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A condition where the immediate delivery price is higher than the future delivery price. Opposite of Contango. |
| 87 | CONTANGO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A condition where the future delivery price is higher than the immediate delivery price. Opposite of Backwardation. |
| 88 | HOLIDAY |  |  |  |  | ✓ | ✓ | ✓ | ✓ | In Development |
| 89 | PRE_OPENING |  |  |  |  | ✓ |  |  |  | The period of time prior to the market opening time (7:00 A.M. - 9:30 A.M.) during which orders are entered into the market for the Opening. |
| 90 | POST_FULL |  |  |  |  |  |  |  |  | false |
| 91 | POST_RESTRICTED |  |  |  |  |  |  |  |  | false |
| 92 | CLOSING_AUCTION |  |  |  |  |  |  |  |  | false |
| 93 | BATCH |  |  |  |  |  |  |  |  | false |
| 94 | TRADING |  |  |  |  |  |  |  |  | false |
| 95 | INTERMARKET_SWEEP |  |  |  |  | ✓ | ✓ | ✓ | ✓ | A trade resulting from an Intermarket Sweep Order Execution. For more information on intermarket sweeps, please see the SEC NMS regulation (June 29, 2005 - PDF).From that report:"The intermarket sweep exception enables trading centers that receive sweep orders to execute those orders immediately, without waiting for betterpriced quotations in other markets to be updated." |
| 96 | DERIVATIVE |  |  |  |  | ✓ | ✓ | ✓ | * | Derivatively priced. |
| 97 | REOPENING |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Market center re-opening prints. |
| 98 | CLOSING |  |  |  |  | ✓ | ✓ | ✓ | * | Market center closing prints. Can be used to get closing auction information for exchanges that report it, such as NYSE. |
| 99 | CAPELECTION |  |  |  |  | ✓ | ✓ | ✓ |  | CTA Docs 78, 110, 111, 113 & 136 Redefined: Existing code 'I' in the Sale Condition field to denote the following change in value: From - Cap Election Trade To - Odd Lot Trade. A trade resulting from an sweep execution where CAP orders were elected and executed outside the best bid or affer and appear as repeat trades. DEL |
| 100 | SPOT_SETTLEMENT |  |  |  |  | ✓ | ✓ | ✓ | ✓ | false |
| 101 | BASIS_HIGH |  |  |  |  | ✓ | ✓ | ✓ |  | false |
| 102 | BASIS_LOW |  |  |  |  | ✓ | ✓ | ✓ |  | false |
| 103 | YIELD |  |  |  |  |  |  |  |  | Applies to bid and ask yield updates for Cantor Treasuries |
| 104 | PRICE_VARIATION |  |  |  |  |  |  |  |  | false |
| 105 | CONTINGENT_TRADE |  |  |  |  | ✓ |  |  |  | Effective July 2015 ~ A Sale Condition used to identify a transaction where the execution of the transaction is contingent upon some event.SIAC Trader Update: February 25, 2015 (PDF) Previously: StockOption |
| 106 | STOPPED_IM |  |  |  |  | ✓ | ✓ | ✓ |  | Transaction order which was stopped at a price that did not constitute a Trade-Through on another market. Valid trade do not update last |
| 107 | BENCHMARK |  |  |  |  |  |  |  | ✓ | This condition will be assigned for Tapes A/B and UTP when no Trade Through Exempt reason is given, and the Trade Through Exempt indicator is set. For Tapes A/B and UTP, these trades are eligible to update O/H/L/L/V. For OPRA, these trades only update volume. |
| 108 | TRADE_THRU_EXEMPT |  |  |  |  |  |  |  |  | true,This condition will be assigned for Tapes A/B and UTP when no Trade Through Exempt reason is given, and the Trade Through Exempt indicator is set. For Tapes A/B and UTP, these trades are eligible to update O/H/L/L/V. For OPRA, these trades only update volume. |
| 109 | IMPLIED |  |  |  |  | ✓ |  |  |  | These trades are result of a spread trade. The exchange sends a leg price on each future for spread transactions. These trades do not update O/H/L/L but they update volume. We are now sending these spread trades for Globex exchanges: CME, NYMEX, COMEX, CBOT, MGE, KCBT and DME. |
| 110 | OTC |  |  |  |  |  |  |  |  | false |
| 111 | MKT_SUPERVISION |  |  |  |  |  |  |  |  | false |
| 112 | RESERVED_77 |  |  |  |  |  |  |  |  | false |
| 113 | RESERVED_91 |  |  |  |  |  |  |  |  | false |
| 114 | CONTINGENT_UTP |  |  |  |  |  |  |  |  |  |
| 115 | ODD_LOT |  |  |  |  | ✓ |  |  |  | This indicates any trade with size between 1-99. |
| 116 | RESERVED_89 |  |  |  |  |  |  |  |  | false |
| 117 | CORRECTED_CS_LAST |  |  |  |  |  | ✓ | ✓ | ✓ | This allows for a mechanism to correct the official close on the consolidated tape. |
| 118 | OPRA_EXT_HOURS |  |  |  |  |  |  |  |  | OPRA extended trading hours session. Equivalent to the OPRA "Session Indicator" with ASCII value of 'X' (Pre-Market extended hours trading session)(Obselete, see condition 148). |
| 119 | RESERVED_78 |  |  |  |  |  |  |  |  | false |
| 120 | RESERVED_81 |  |  |  |  |  |  |  |  | false |
| 121 | RESERVED_84 |  |  |  |  |  |  |  |  | false |
| 122 | RESERVED_878 |  |  |  |  |  |  |  |  | false |
| 123 | RESERVED_90 |  |  |  |  |  |  |  |  | false |
| 124 | QUALIFIED_CONTINGENT_TRADE |  |  |  |  | ✓ |  |  |  | Effective July 2015 ~ A transaction consisting of two or more component orders, executed as agent or principal, that meets each of the following elements: At least one component order is for an NMS stock. All components are effected with a product or price contingency that either has been agreed to by the respective counterparties or arranged for by a broker-dealer as principal or agent. The execution of one component is contingent upon the execution of all other components at or near the same time. The specific relationship between the component orders (e.g. the spread between the prices of the component orders) is determined at the time the contingent order is placed. The component orders bear a derivative relationship to one another, represent different classes of shares of the same issuer, or involve the securities of participants in mergers or with intentions to |
| 125 | SINGLE_LEG_AUCTION_NON_ISO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure period. Such auctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism. |
| 126 | SINGLE_LEG_AUCTION_ISO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an Intermarket Sweep electronic order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure period. Suchauctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism marked as ISO. |
| 127 | SINGLE_LEG_CROSS_NON_ISO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic order which was "stopped" at a price and traded in a two sided crossing mechanism that does not go through an exposure period. Such crossing mechanisms include and not limited to Customer to Customer Cross and QCC with a single option leg. |
| 128 | SINGLE_LEG_CROSS_ISO |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an Intermarket Sweep electronic order which was "stopped" at a price and traded in a two sided crossing mechanism that does not go through an exposure period. Such crossing mechanisms include and not limited to Customer to Customer Cross. |
| 129 | SINGLE_LEG_FLOOR_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents a non-electronic trade executed on a trading floor. Execution of Paired and Non-Paired Auctions and Cross orders on an exchange floor are also included in this category. |
| 130 | MULTI_LEG_AUTOELEC_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transactionrepresents an electronic execution of a multi leg order traded in a complex order book. |
| 131 | MULTI_LEG_AUCTION |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure period in a complex order book. Such auctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism. |
| 132 | MULTI_LEG_CROSS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg order which was "stopped" at a price and traded in a two sided crossing mechanism that does not go through an exposure period. Such crossing mechanisms include and not limited to Customer to Customer Cross and QCC with two or more options legs. |
| 133 | MULTI_LEG_FLOOR_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents a non-electronic multi leg order trade executed against other multi-leg order(s) on a trading floor. Execution of Paired and Non-Paired Auctions and Cross orders on an exchange floor are also included in this category. |
| 134 | ML_AUTO_ELEC_TRADE_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents an electronic execution of a multi Leg order traded against single leg orders/quotes. |
| 135 | STOCK_OPTIONS_AUCTION |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg stock/options order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure period in a complex order book. Such auctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism. |
| 136 | ML_AUCTION_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure period and trades against single leg orders/ quotes. Such auctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism. |
| 137 | ML_FLOOR_TRADE_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents a non-electronic multi leg order trade executed on a trading floor against single leg orders/ quotes. Execution of Paired and Non-Paired Auctions on an exchange floor are also included in this category. |
| 138 | STK_OPT_AUTO_ELEC_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents an electronic execution of a multi leg stock/options order traded in a complex order book. |
| 139 | STOCK_OPTIONS_CROSS |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg stock/options order which was "stopped" at a price and traded in a two sided crossing mechanism that does not go through an exposure period. Such crossing mechanisms include and not limited to Customer to Customer Cross. |
| 140 | STOCK_OPTIONS_FLOOR_TRADE |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents a non-electronic multi leg order stock/options trade executed on a trading floor in a Complex order book. Execution of Paired and Non-Paired Auctions and Cross orders on an exchange floor are also included in this category. |
| 141 | STK_OPT_AE_TRD_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents an electronic execution of a multi Leg stock/options order traded against single leg orders/quotes. |
| 142 | STK_OPT_AUCTION_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction was the execution of an electronic multi leg stock/options order which was "stopped" at a price and traded in a two sided auction mechanism that goes through an exposure periodand trades against single leg orders/ quotes. Such auctions mechanisms include and not limited to Price Improvement, Facilitation or Solicitation Mechanism. |
| 143 | STK_OPT_FLOOR_TRADE_AGSL |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents a non-electronic multi leg stock/options order trade executed on a trading floor against single leg orders/ quotes. Execution of Paired and Non-Paired Auctions on an exchange floor are also included in this category. |
| 144 | ML_FLOOR_TRADE_OF_PP |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Transaction represents execution of a proprietary product non-electronic multi leg order with at least 3 legs. The trade price may be outside the current NBBO. |
| 145 | BID_AGGRESSOR |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Aggressor of the trade is on the buy side. |
| 146 | ASK_AGGRESSOR |  |  |  |  | ✓ | ✓ | ✓ | ✓ | Aggressor of the trade is on the sell side. |
| 147 | MULTILAT_COMP_TR_PDP |  |  |  |  | ✓ |  |  |  | Transaction represents an execution in a proprietary product done as part of a multilateral compression. Trades are executed outside of regular trading hours at prices derived from end of day markets. |
| 148 | EXTENDED_HOURS_TRADE |  |  |  |  | ✓ |  |  |  | Transaction represents a trade that was executed outside of regular market hours. |
