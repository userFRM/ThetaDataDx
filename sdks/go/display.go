package thetadatadx

import "fmt"

// ── Time / Date helpers ──

// TimeStr formats milliseconds-of-day as "HH:MM:SS.mmm".
func TimeStr(msOfDay int) string {
	ms := uint32(msOfDay)
	totalSecs := ms / 1000
	millis := ms % 1000
	h := totalSecs / 3600
	m := (totalSecs % 3600) / 60
	s := totalSecs % 60
	return fmt.Sprintf("%02d:%02d:%02d.%03d", h, m, s, millis)
}

// DateStr formats a YYYYMMDD integer as "YYYY-MM-DD".
func DateStr(date int) string {
	y := date / 10000
	m := (date % 10000) / 100
	d := date % 100
	return fmt.Sprintf("%04d-%02d-%02d", y, m, d)
}

// RightStr returns "C" for calls (ASCII 67), "P" for puts (ASCII 80), else "".
func RightStr(code int32) string {
	switch code {
	case 67:
		return "C"
	case 80:
		return "P"
	default:
		return ""
	}
}

// ── Exchange lookup ──

type exchangeEntry struct {
	Name   string
	Symbol string
}

var exchanges = [78]exchangeEntry{
	{Name: "NanexComp", Symbol: "COMP"},
	{Name: "NasdaqExchange", Symbol: "NQEX"},
	{Name: "NasdaqAlternativeDisplayFacility", Symbol: "NQAD"},
	{Name: "NewYorkStockExchange", Symbol: "NYSE"},
	{Name: "AmericanStockExchange", Symbol: "AMEX"},
	{Name: "ChicagoBoardOptionsExchange", Symbol: "CBOE"},
	{Name: "InternationalSecuritiesExchange", Symbol: "ISEX"},
	{Name: "NYSEARCA(Pacific)", Symbol: "PACF"},
	{Name: "NationalStockExchange(Cincinnati)", Symbol: "CINC"},
	{Name: "PhiladelphiaStockExchange", Symbol: "PHIL"},
	{Name: "OptionsPricingReportingAuthority", Symbol: "OPRA"},
	{Name: "BostonStock/OptionsExchange", Symbol: "BOST"},
	{Name: "NasdaqGlobal+SelectMarket(NMS)", Symbol: "NQNM"},
	{Name: "NasdaqCapitalMarket(SmallCap)", Symbol: "NQSC"},
	{Name: "NasdaqBulletinBoard", Symbol: "NQBB"},
	{Name: "NasdaqOTC", Symbol: "NQPK"},
	{Name: "NasdaqIndexes(GIDS)", Symbol: "NQIX"},
	{Name: "ChicagoStockExchange", Symbol: "CHIC"},
	{Name: "TorontoStockExchange", Symbol: "TSE"},
	{Name: "CanadianVentureExchange", Symbol: "CDNX"},
	{Name: "ChicagoMercantileExchange", Symbol: "CME"},
	{Name: "NewYorkBoardofTrade", Symbol: "NYBT"},
	{Name: "ISEMercury", Symbol: "MRCY"},
	{Name: "COMEX(divisionofNYMEX)", Symbol: "COMX"},
	{Name: "ChicagoBoardofTrade", Symbol: "CBOT"},
	{Name: "NewYorkMercantileExchange", Symbol: "NYMX"},
	{Name: "KansasCityBoardofTrade", Symbol: "KCBT"},
	{Name: "MinneapolisGrainExchange", Symbol: "MGEX"},
	{Name: "NYSE/ARCABonds", Symbol: "NYBO"},
	{Name: "NasdaqBasic", Symbol: "NQBS"},
	{Name: "DowJonesIndices", Symbol: "DOWJ"},
	{Name: "ISEGemini", Symbol: "GEMI"},
	{Name: "SingaporeInternationalMonetaryExchange", Symbol: "SIMX"},
	{Name: "LondonStockExchange", Symbol: "FTSE"},
	{Name: "Eurex", Symbol: "EURX"},
	{Name: "ImpliedPrice", Symbol: "IMPL"},
	{Name: "DataTransmissionNetwork", Symbol: "DTN"},
	{Name: "LondonMetalsExchangeMatchedTrades", Symbol: "LMT"},
	{Name: "LondonMetalsExchange", Symbol: "LME"},
	{Name: "IntercontinentalExchange(IPE)", Symbol: "IPEX"},
	{Name: "NasdaqMutualFunds(MFDS)", Symbol: "NQMF"},
	{Name: "COMEXClearport", Symbol: "fcec"},
	{Name: "CBOEC2OptionExchange", Symbol: "C2"},
	{Name: "MiamiExchange", Symbol: "MIAX"},
	{Name: "NYMEXClearport", Symbol: "CLRP"},
	{Name: "Barclays", Symbol: "BARK"},
	{Name: "MiamiEmeraldOptionsExchange", Symbol: "EMLD"},
	{Name: "NASDAQBoston", Symbol: "NQBX"},
	{Name: "HotSpotEurexUS", Symbol: "HOTS"},
	{Name: "EurexUS", Symbol: "EUUS"},
	{Name: "EurexEU", Symbol: "EUEU"},
	{Name: "EuronextCommodities", Symbol: "ENCM"},
	{Name: "EuronextIndexDerivatives", Symbol: "ENID"},
	{Name: "EuronextInterestRates", Symbol: "ENIR"},
	{Name: "CBOEFuturesExchange", Symbol: "CFE"},
	{Name: "PhiladelphiaBoardofTrade", Symbol: "PBOT"},
	{Name: "FCME", Symbol: "CMEFloor"},
	{Name: "FINRA/NASDAQTradeReportingFacility", Symbol: "NQNX"},
	{Name: "BSETradeReportingFacility", Symbol: "BTRF"},
	{Name: "NYSETradeReportingFacility", Symbol: "NTRF"},
	{Name: "BATSTrading", Symbol: "BATS"},
	{Name: "CBOTFloor", Symbol: "FCBT"},
	{Name: "PinkSheets", Symbol: "PINK"},
	{Name: "BATSYExchange", Symbol: "BATY"},
	{Name: "DirectEdgeA", Symbol: "EDGE"},
	{Name: "DirectEdgeX", Symbol: "EDGX"},
	{Name: "RussellIndexes", Symbol: "RUSL"},
	{Name: "CMEIndexes", Symbol: "CMEX"},
	{Name: "InvestorsExchange", Symbol: "IEX"},
	{Name: "MiamiPearlOptionsExchange", Symbol: "PERL"},
	{Name: "LondonStockExchange", Symbol: "LSE"},
	{Name: "NYSEGlobalIndexFeed", Symbol: "GIF"},
	{Name: "TSXIndexes", Symbol: "TSIX"},
	{Name: "MembersExchange", Symbol: "MEMX"},
	{Name: "CBOECGI", Symbol: "CGI"},
	{Name: "LongTermStockExchange", Symbol: "LTSE"},
	{Name: "MIAXSapphire", Symbol: "SPHR"},
	{Name: "24XNationalExchange", Symbol: "24X"},
}

// ExchangeName returns the exchange name for the given code, or "UNKNOWN".
func ExchangeName(code int) string {
	if code >= 0 && code < len(exchanges) {
		return exchanges[code].Name
	}
	return "UNKNOWN"
}

// ExchangeSymbol returns the exchange symbol for the given code, or "UNKNOWN".
func ExchangeSymbol(code int) string {
	if code >= 0 && code < len(exchanges) {
		return exchanges[code].Symbol
	}
	return "UNKNOWN"
}

// ── Trade condition lookup ──

var conditionNames = [149]string{
	"REGULAR", "FORMT", "OUTOFSEQ", "AVGPRC", "AVGPRC_NASDAQ",
	"OPENREPORTLATE", "OPENREPORTOUTOFSEQ", "OPENREPORTINSEQ",
	"PRIORREFERENCEPRICE", "NEXTDAYSALE", "BUNCHED", "CASHSALE",
	"SELLER", "SOLDLAST", "RULE127", "BUNCHEDSOLD", "NONBOARDLOT",
	"POSIT", "AUTOEXECUTION", "HALT", "DELAYED", "REOPEN",
	"ACQUISITION", "CASHMARKET", "NEXTDAYMARKET", "BURSTBASKET",
	"OPENDETAIL", "INTRADETAIL", "BASKETONCLOSE", "RULE155",
	"DISTRIBUTION", "SPLIT", "REGULARSETTLE", "CUSTOMBASKETCROSS",
	"ADJTERMS", "SPREAD", "STRADDLE", "BUYWRITE", "COMBO", "STPD",
	"CANC", "CANCLAST", "CANCOPEN", "CANCONLY", "CANCSTPD",
	"MATCHCROSS", "FASTMARKET", "NOMINAL", "CABINET", "BLANKPRICE",
	"NOTSPECIFIED", "MCOFFICIALCLOSE", "SPECIALTERMS", "CONTINGENTORDER",
	"INTERNALCROSS", "STOPPEDREGULAR", "STOPPEDSOLDLAST", "STOPPEDOUTOFSEQ",
	"BASIS", "VWAP", "SPECIALSESSION", "NANEXADMIN", "OPENREPORT",
	"MARKETONCLOSE", "SETTLEPRICE", "OUTOFSEQPREMKT", "MCOFFICIALOPEN",
	"FUTURESSPREAD", "OPENRANGE", "CLOSERANGE", "NOMINALCABINET",
	"CHANGINGTRANS", "CHANGINGTRANSCAB", "NOMINALUPDATE", "PITSETTLEMENT",
	"BLOCKTRADE", "EXGFORPHYSICAL", "VOLUMEADJUSTMENT", "VOLATILITYTRADE",
	"YELLOWFLAG", "FLOORPRICE", "OFFICIALPRICE", "UNOFFICIALPRICE",
	"MIDBIDASKPRICE", "ENDSESSIONHIGH", "ENDSESSIONLOW", "BACKWARDATION",
	"CONTANGO", "HOLIDAY", "PREOPENING", "POSTFULL", "POSTRESTRICTED",
	"CLOSINGAUCTION", "BATCH", "TRADING", "INTERMARKETSWEEP",
	"DERIVATIVE", "REOPENING", "CLOSING", "CAPELECTION", "SPOTSETTLEMENT",
	"BASISHIGH", "BASISLOW", "YIELD", "PRICEVARIATION",
	"CONTINGENTTRADEFORMERLYSTOCKOPTION", "STOPPEDIM", "BENCHMARK",
	"TRADETHRUEXEMPT", "IMPLIED", "OTC", "MKTSUPERVISION",
	"RESERVED_77", "RESERVED_91", "CONTINGENTUTP", "ODDLOT",
	"RESERVED_89", "CORRECTEDCSLAST", "OPRAEXTHOURS", "RESERVED_78",
	"RESERVED_81", "RESERVED_84", "RESERVED_878", "RESERVED_90",
	"QUALIFIEDCONTINGENTTRADE", "SINGLELEGAUCTIONNONISO",
	"SINGLELEGAUCTIONISO", "SINGLELEGCROSSNONISO", "SINGLELEGCROSSISO",
	"SINGLELEGFLOORTRADE", "MULTILEGAUTOELECTRONICTRADE",
	"MULTILEGAUCTION", "MULTILEGCROSS", "MULTILEGFLOORTRADE",
	"MULTILEGAUTOELECTRADEAGAINSTSINGLELEG", "STOCKOPTIONSAUCTION",
	"MULTILEGAUCTIONAGAINSTSINGLELEG",
	"MULTILEGFLOORTRADEAGAINSTSINGLELEG", "STOCKOPTIONSAUTOELECTRADE",
	"STOCKOPTIONSCROSS", "STOCKOPTIONSFLOORTRADE",
	"STOCKOPTIONSAUTOELECTRADEAGAINSTSINGLELEG",
	"STOCKOPTIONSAUCTIONAGAINSTSINGLELEG",
	"STOCKOPTIONSFLOORTRADEAGAINSTSINGLELEG",
	"MULTILEGFLOORTRADEOFPROPRIETARYPRODUCTS", "BIDAGGRESSOR",
	"ASKAGGRESSOR",
	"MULTILATERALCOMPRESSIONTRADEOFPROPRIETARYDATAPRODUCTS",
	"EXTENDEDHOURSTRADE",
}

// ConditionName returns the trade condition name for the given code, or "UNKNOWN".
func ConditionName(code int) string {
	if code >= 0 && code < len(conditionNames) {
		return conditionNames[code]
	}
	return "UNKNOWN"
}

// ── Quote condition lookup ──

var quoteConditionNames = [75]string{
	"REGULAR", "BID_ASK_AUTO_EXEC", "ROTATION", "SPECIALIST_ASK",
	"SPECIALIST_BID", "LOCKED", "FAST_MARKET", "SPECIALIST_BID_ASK",
	"ONE_SIDE", "OPENING_QUOTE", "CLOSING_QUOTE", "MARKET_MAKER_CLOSED",
	"DEPTH_ON_ASK", "DEPTH_ON_BID", "DEPTH_ON_BID_ASK", "TIER_3",
	"CROSSED", "HALTED", "OPERATIONAL_HALT", "NEWS_OUT", "NEWS_PENDING",
	"NON_FIRM", "DUE_TO_RELATED", "RESUME", "NO_MARKET_MAKERS",
	"ORDER_IMBALANCE", "ORDER_INFLUX", "INDICATED", "PRE_OPEN",
	"IN_VIEW_OF_COMMON", "RELATED_NEWS_PENDING", "RELATED_NEWS_OUT",
	"ADDITIONAL_INFO", "RELATED_ADD_INFO", "NO_OPEN_RESUME", "DELETED",
	"REGULATORY_HALT", "SEC_SUSPENSION", "NON_COMLIANCE",
	"FILINGS_NOT_CURRENT", "CATS_HALTED", "CATS", "EX_DIV_OR_SPLIT",
	"UNASSIGNED", "INSIDE_OPEN", "INSIDE_CLOSED", "OFFER_WANTED",
	"BID_WANTED", "CASH", "INACTIVE", "NATIONAL_BBO", "NOMINAL",
	"CABINET", "NOMINAL_CABINET", "BLANK_PRICE", "SLOW_BID_ASK",
	"SLOW_LIST", "SLOW_BID", "SLOW_ASK", "BID_OFFER_WANTED",
	"SUBPENNY", "NON_BBO", "SPECIAL_OPEN", "BENCHMARK", "IMPLIED",
	"EXCHANGE_BEST", "MKT_WIDE_HALT_1", "MKT_WIDE_HALT_2",
	"MKT_WIDE_HALT_3", "ON_DEMAND_AUCTION", "NON_FIRM_BID",
	"NON_FIRM_ASK", "RETAIL_BID", "RETAIL_ASK", "RETAIL_QTE",
}

// QuoteConditionName returns the quote condition name for the given code, or "UNKNOWN".
func QuoteConditionName(code int) string {
	if code >= 0 && code < len(quoteConditionNames) {
		return quoteConditionNames[code]
	}
	return "UNKNOWN"
}
